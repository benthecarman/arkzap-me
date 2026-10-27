use anyhow::Context;
use axum::extract::DefaultBodyLimit;
use axum::http::Method;
use axum::middleware;
use axum::routing::get;
use axum::{http, Extension, Router};
use bark_rest_client::models::LightningReceiveInfo;
use clap::Parser;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use log::{error, info, warn};
use nostr::key::Keys;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;

use crate::arkade::ArkadeClient;
use crate::barkd::{receive_allows_expiry, revealed_preimage, BarkdClient};
use crate::config::*;
use crate::models::arkade_invoice::ArkadeInvoice;
use crate::models::custom_address::CustomAddressInvoice;
use crate::models::invoice::{Invoice, InvoiceState};
use crate::rate_limit::{rate_limit_middleware, RateLimiter};
use crate::routes::*;

mod arkade;
mod barkd;
mod config;
mod models;
mod rate_limit;
mod routes;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

#[derive(Clone)]
pub struct State {
    pub db_pool: Pool<ConnectionManager<PgConnection>>,
    pub keys: Keys,
    pub barkd: Arc<BarkdClient>,
    pub arkade: Option<Arc<ArkadeClient>>,

    // -- config options --
    pub domain: String,
    pub network: bitcoin::Network,
    pub min_sendable: u64,
    pub max_sendable: u64,
    pub custom_address_fee_sats: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    pretty_env_logger::try_init()?;
    let config: Config = Config::parse();

    let keys = Keys::from_str(&config.nsec)?;

    let manager = ConnectionManager::<PgConnection>::new(config.pg_url.clone());
    let db_pool = Pool::builder()
        .max_size(10) // should be a multiple of 100, our database connection limit
        .test_on_check_out(true)
        .build(manager)
        .expect("Unable to build DB connection pool");
    run_migrations(&db_pool)?;

    let barkd = Arc::new(BarkdClient::new(
        config.barkd_url.clone(),
        config.barkd_token.clone(),
    )?);
    let arkade = if config.disable_arkade {
        None
    } else {
        Some(Arc::new(
            ArkadeClient::new(
                db_pool.clone(),
                config
                    .arkade_xpriv
                    .clone()
                    .context("LNURL_ARKADE_XPRIV is required unless Arkade support is disabled")?,
                config.arkade_server_url.clone().context(
                    "LNURL_ARKADE_SERVER_URL is required unless Arkade support is disabled",
                )?,
                config.arkade_boltz_url.clone().context(
                    "LNURL_ARKADE_BOLTZ_URL is required unless Arkade support is disabled",
                )?,
                config.arkade_esplora_url.clone(),
                config.network,
                config.arkade_invoice_expiry_secs,
                config.request_timeout_seconds,
            )
            .await?,
        ))
    };
    let rate_limiter = Arc::new(RateLimiter::new(config.rate_limit_per_minute));
    let request_timeout = Duration::from_secs(config.request_timeout_seconds);
    let max_request_body_bytes = config.max_request_body_bytes;

    let state = State {
        db_pool: db_pool.clone(),
        keys: keys.clone(),
        barkd,
        arkade,
        domain: config.domain,
        network: config.network,
        min_sendable: config.min_sendable,
        max_sendable: config.max_sendable,
        custom_address_fee_sats: config.custom_address_fee_sats,
    };

    tokio::spawn(claim_paid_invoices(state.clone()));

    let addr: std::net::SocketAddr = format!("{}:{}", config.bind, config.port)
        .parse()
        .expect("Failed to parse bind/port for webserver");

    println!("Webserver running on http://{addr}");

    let server_router = Router::new()
        .route("/", get(root))
        .route("/health-check", get(health_check))
        .route("/get-invoice/{ark_address}", get(get_invoice))
        .route("/verify/{address}/{pay_hash}", get(verify))
        .route("/.well-known/lnurlp/{ark_address}", get(get_lnurl_pay))
        .route(
            "/custom-addresses/auth-message",
            get(custom_address_auth_message),
        )
        .route(
            "/custom-addresses",
            axum::routing::post(create_custom_address_invoice),
        )
        .route("/custom-addresses/{id}", get(get_custom_address_invoice))
        .fallback(fallback)
        .layer(Extension(state.clone()))
        .layer(middleware::from_fn_with_state(
            rate_limiter,
            rate_limit_middleware,
        ))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers([http::header::CONTENT_TYPE, http::header::AUTHORIZATION])
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::DELETE,
                    Method::OPTIONS,
                ]),
        )
        .layer(TimeoutLayer::with_status_code(
            http::StatusCode::REQUEST_TIMEOUT,
            request_timeout,
        ))
        .layer(DefaultBodyLimit::max(max_request_body_bytes));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("failed to bind webserver listener")?;
    let server = axum::serve(
        listener,
        server_router.into_make_service_with_connect_info::<SocketAddr>(),
    );

    // todo Invoice event stream for zaps

    let graceful = server.with_graceful_shutdown(async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to create Ctrl+C shutdown signal");
    });

    // Await the server to receive the shutdown signal
    if let Err(e) = graceful.await {
        eprintln!("shutdown error: {e}");
    }

    Ok(())
}

fn run_migrations(pool: &Pool<ConnectionManager<PgConnection>>) -> anyhow::Result<()> {
    let mut conn = pool
        .get()
        .context("failed to get DB connection for migrations")?;
    conn.run_pending_migrations(MIGRATIONS)
        .map_err(|e| anyhow::anyhow!("failed to run database migrations: {e}"))?;
    Ok(())
}

async fn claim_paid_invoices(state: State) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        interval.tick().await;
        if let Err(e) = claim_paid_invoices_once(&state).await {
            error!("Unable to claim paid invoices: {e:#}");
        }
    }
}

async fn claim_paid_invoices_once(state: &State) -> anyhow::Result<()> {
    claim_paid_bark_invoices_once(state).await?;
    if state.arkade.is_some() {
        claim_paid_arkade_invoices_once(state).await?;
    }
    Ok(())
}

async fn claim_paid_bark_invoices_once(state: &State) -> anyhow::Result<()> {
    claim_paid_custom_address_invoices_once(state).await?;

    let invoices = {
        let mut conn = state.db_pool.get()?;
        Invoice::get_by_state(&mut conn, InvoiceState::Pending as i32)?
    };

    let bark_pending_receives = match state.barkd.pending_receives().await {
        Ok(receives) => Some(
            receives
                .into_iter()
                .map(|receive| (receive.payment_hash.to_string(), receive))
                .collect::<HashMap<_, _>>(),
        ),
        Err(e) => {
            warn!(
                "Unable to list pending barkd receives, falling back to per-invoice checks: {e:#}"
            );
            None
        }
    };

    for invoice in invoices {
        let payment_hash = invoice_payment_hash(&invoice);
        if let Some(receive) = bark_pending_receives
            .as_ref()
            .and_then(|pending_receives| pending_receives.get(&payment_hash))
        {
            if let Err(e) = apply_invoice_receive_status(state, &invoice, &payment_hash, receive) {
                error!("Unable to claim invoice: {e:#}");
            }
            continue;
        }

        if let Err(e) = claim_invoice_if_paid(state, invoice, payment_hash).await {
            error!("Unable to claim invoice: {e:#}");
        }
    }

    Ok(())
}

async fn claim_paid_custom_address_invoices_once(state: &State) -> anyhow::Result<()> {
    let invoices = {
        let mut conn = state.db_pool.get()?;
        CustomAddressInvoice::get_by_state(&mut conn, InvoiceState::Pending as i32)?
    };

    if !invoices.is_empty() {
        info!(
            "Checking {} pending custom address invoice(s) for payment",
            invoices.len()
        );
    }

    for invoice in invoices {
        let payment_hash = invoice_payment_hash(&invoice);
        if let Err(e) = claim_custom_address_invoice_if_paid(state, invoice, payment_hash).await {
            error!("Unable to claim custom address invoice: {e:#}");
        }
    }

    Ok(())
}

async fn claim_paid_arkade_invoices_once(state: &State) -> anyhow::Result<()> {
    let arkade = state
        .arkade
        .as_ref()
        .context("Arkade support is disabled")?;
    let invoices = {
        let mut conn = state.db_pool.get()?;
        ArkadeInvoice::get_by_state(&mut conn, InvoiceState::Pending as i32)?
    };

    for invoice in invoices {
        let payment_hash = invoice_payment_hash(&invoice);
        if invoice_has_expired(&invoice) {
            let mut conn = state.db_pool.get()?;
            if invoice.mark_cancelled(&mut conn)? {
                info!(
                    "Cancelled expired Arkade invoice {} payment_hash={} amount_msats={}",
                    invoice.id, payment_hash, invoice.amount_msats
                );
            }
            continue;
        }

        match arkade.claim_receive(&invoice.swap_id).await {
            Ok(preimage) => {
                let mut conn = state.db_pool.get()?;
                if invoice.mark_settled(&mut conn, hex::encode(preimage))? {
                    info!(
                        "Claimed Arkade invoice {} payment_hash={} amount_msats={} swap_id={}",
                        invoice.id, payment_hash, invoice.amount_msats, invoice.swap_id
                    );
                }
            }
            Err(e) => {
                error!(
                    "Unable to claim Arkade invoice {} payment_hash={} swap_id={}: {e:#}",
                    invoice.id, payment_hash, invoice.swap_id
                );
            }
        }
    }

    Ok(())
}

trait StoredInvoice {
    fn expires_at(&self) -> Option<chrono::NaiveDateTime>;
    fn bolt11(&self) -> lightning_invoice::Bolt11Invoice;
    fn payment_hash(&self) -> Option<&str>;
}

impl StoredInvoice for Invoice {
    fn expires_at(&self) -> Option<chrono::NaiveDateTime> {
        self.expires_at
    }

    fn bolt11(&self) -> lightning_invoice::Bolt11Invoice {
        self.bolt11()
    }

    fn payment_hash(&self) -> Option<&str> {
        self.payment_hash.as_deref()
    }
}

impl StoredInvoice for ArkadeInvoice {
    fn expires_at(&self) -> Option<chrono::NaiveDateTime> {
        self.expires_at
    }

    fn bolt11(&self) -> lightning_invoice::Bolt11Invoice {
        self.bolt11()
    }

    fn payment_hash(&self) -> Option<&str> {
        self.payment_hash.as_deref()
    }
}

impl StoredInvoice for CustomAddressInvoice {
    fn expires_at(&self) -> Option<chrono::NaiveDateTime> {
        Some(self.expires_at)
    }

    fn bolt11(&self) -> lightning_invoice::Bolt11Invoice {
        self.bolt11()
    }

    fn payment_hash(&self) -> Option<&str> {
        self.payment_hash.as_deref()
    }
}

fn invoice_has_expired(invoice: &impl StoredInvoice) -> bool {
    invoice
        .expires_at()
        .is_some_and(|expires_at| expires_at <= chrono::Utc::now().naive_utc())
        || invoice.bolt11().is_expired()
}

fn invoice_payment_hash(invoice: &impl StoredInvoice) -> String {
    match invoice.payment_hash() {
        Some(payment_hash) => payment_hash.to_string(),
        None => invoice.bolt11().payment_hash().to_string(),
    }
}

async fn claim_invoice_if_paid(
    state: &State,
    invoice: Invoice,
    payment_hash: String,
) -> anyhow::Result<()> {
    info!(
        "Checking claim status for invoice {} payment_hash={} amount_msats={}",
        invoice.id, payment_hash, invoice.amount_msats
    );

    let receive = state
        .barkd
        .receive_status(&payment_hash)
        .await
        .with_context(|| {
            format!(
                "failed to get lightning receive status for invoice {}",
                invoice.id
            )
        })?;

    let Some(receive) = receive else {
        warn!("Barkd has no receive status for invoice {}", invoice.id);
        cancel_invoice_if_expired(state, &invoice, &payment_hash)?;
        return Ok(());
    };

    apply_invoice_receive_status(state, &invoice, &payment_hash, &receive)
}

fn apply_invoice_receive_status(
    state: &State,
    invoice: &Invoice,
    payment_hash: &str,
    receive: &LightningReceiveInfo,
) -> anyhow::Result<()> {
    if let Some(preimage) = revealed_preimage(receive) {
        info!(
            "Barkd receive ready to claim for invoice {} payment_hash={} \
             state={} settled_at={:?}",
            invoice.id, payment_hash, receive.state, receive.settled_at
        );

        let mut conn = state.db_pool.get()?;
        if invoice.mark_settled(&mut conn, preimage.to_string())? {
            info!(
                "Claimed invoice {} payment_hash={} amount_msats={} settled_at={:?}",
                invoice.id, payment_hash, invoice.amount_msats, receive.settled_at
            );
        }
        return Ok(());
    }

    if receive_allows_expiry(receive) {
        cancel_invoice_if_expired(state, invoice, payment_hash)?;
    }

    Ok(())
}

fn cancel_invoice_if_expired(
    state: &State,
    invoice: &Invoice,
    payment_hash: &str,
) -> anyhow::Result<()> {
    if invoice_has_expired(invoice) {
        let mut conn = state.db_pool.get()?;
        if invoice.mark_cancelled(&mut conn)? {
            info!(
                "Cancelled expired invoice {} payment_hash={} amount_msats={}",
                invoice.id, payment_hash, invoice.amount_msats
            );
        }
    }

    Ok(())
}

async fn claim_custom_address_invoice_if_paid(
    state: &State,
    invoice: CustomAddressInvoice,
    payment_hash: String,
) -> anyhow::Result<()> {
    info!(
        "Checking custom address invoice {} name={} payment_hash={} amount_msats={} fee_receive_address={}",
        invoice.id, invoice.name, payment_hash, invoice.amount_msats, invoice.fee_receive_address
    );

    let receive = state
        .barkd
        .receive_status(&payment_hash)
        .await
        .with_context(|| {
            format!(
                "failed to get lightning receive status for custom address invoice {}",
                invoice.id
            )
        })?;

    let Some(receive) = receive else {
        info!(
            "Custom address invoice {} has no Lightning receive status; checking Ark fee payment fee_receive_address={}",
            invoice.id, invoice.fee_receive_address
        );
        if claim_custom_address_invoice_if_ark_paid(state, &invoice).await? {
            return Ok(());
        }
        cancel_custom_address_invoice_if_expired(state, &invoice, &payment_hash)?;
        return Ok(());
    };

    info!(
        "Custom address invoice {} Lightning receive status payment_hash={} state={} settled_at={:?}",
        invoice.id,
        payment_hash,
        receive.state,
        receive.settled_at
    );

    if let Some(preimage) = revealed_preimage(&receive) {
        let mut conn = state.db_pool.get()?;
        if invoice.mark_lightning_settled_and_activate(&mut conn, preimage.to_string())? {
            info!(
                "Activated custom address {} for {} from invoice {} payment_hash={}",
                invoice.name, invoice.ark_address, invoice.id, payment_hash
            );
        } else {
            warn!(
                "Custom address invoice activation was a no-op invoice_id={} name={} payment_hash={}",
                invoice.id, invoice.name, payment_hash
            );
        }
        return Ok(());
    }

    if claim_custom_address_invoice_if_ark_paid(state, &invoice).await? {
        return Ok(());
    }

    if receive_allows_expiry(&receive) {
        cancel_custom_address_invoice_if_expired(state, &invoice, &payment_hash)?;
    }

    Ok(())
}

async fn claim_custom_address_invoice_if_ark_paid(
    state: &State,
    invoice: &CustomAddressInvoice,
) -> anyhow::Result<bool> {
    info!(
        "Checking Ark fee payment for custom address invoice {} name={} fee_receive_address={} amount_sats={}",
        invoice.id,
        invoice.name,
        invoice.fee_receive_address,
        invoice.amount_msats / 1_000
    );
    let ark_payment = state
        .barkd
        .received_ark_payment(
            &invoice.fee_receive_address,
            (invoice.amount_msats / 1_000) as u64,
        )
        .await
        .with_context(|| {
            format!(
                "failed to check Ark receive payment for custom address invoice {}",
                invoice.id
            )
        })?;

    let Some(ark_payment) = ark_payment else {
        info!(
            "No Ark fee payment found for custom address invoice {} fee_receive_address={}",
            invoice.id, invoice.fee_receive_address
        );
        return Ok(false);
    };

    let mut conn = state.db_pool.get()?;
    if invoice.mark_ark_settled_and_activate(&mut conn, ark_payment.reference.clone())? {
        info!(
            "Activated custom address {} for {} from Ark payment to {} reference={} amount_sats={}",
            invoice.name,
            invoice.ark_address,
            invoice.fee_receive_address,
            ark_payment.reference,
            ark_payment.amount_sat
        );
    } else {
        warn!(
            "Custom address invoice Ark activation was a no-op invoice_id={} name={} fee_receive_address={}",
            invoice.id, invoice.name, invoice.fee_receive_address
        );
    }

    Ok(true)
}

fn cancel_custom_address_invoice_if_expired(
    state: &State,
    invoice: &CustomAddressInvoice,
    payment_hash: &str,
) -> anyhow::Result<()> {
    if invoice_has_expired(invoice) {
        let mut conn = state.db_pool.get()?;
        if invoice.mark_cancelled(&mut conn)? {
            info!(
                "Cancelled expired custom address invoice {} payment_hash={} amount_msats={}",
                invoice.id, payment_hash, invoice.amount_msats
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::models::custom_address::{
        CustomAddress, CustomAddressInvoice, NewCustomAddressInvoice,
    };
    use crate::models::invoice::NewInvoice;
    use ark::bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};
    use ark::mailbox::MailboxIdentifier;
    use axum::routing::{get, post};
    use axum::{Extension, Json, Router};
    use bitcoin::hashes::{sha256, Hash};
    use bitcoin::secp256k1::{Secp256k1 as BitcoinSecp256k1, SecretKey as BitcoinSecretKey};
    use chrono::Duration as ChronoDuration;
    use diesel::connection::SimpleConnection;
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Text};
    use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
    use serde_json::{json, Value};
    use std::env;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use tokio::sync::oneshot;

    struct TestSchema {
        database_url: String,
        schema: String,
    }

    impl TestSchema {
        fn new(test_name: &str) -> anyhow::Result<Option<(Self, PgConnection)>> {
            dotenvy::dotenv().ok();

            let Ok(database_url) = env::var("LNURL_TEST_DATABASE_URL") else {
                eprintln!("skipping {test_name}: LNURL_TEST_DATABASE_URL is not set");
                return Ok(None);
            };

            let schema = format!(
                "lnurl_bark_test_{}_{}_{}",
                test_name,
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
            );
            let schema = schema.replace('-', "_");

            let mut conn = PgConnection::establish(&database_url)?;
            conn.batch_execute(&format!(
                r#"CREATE SCHEMA "{schema}"; SET search_path TO "{schema}";"#
            ))?;

            Ok(Some((
                Self {
                    database_url,
                    schema,
                },
                conn,
            )))
        }

        fn set_search_path(&self, conn: &mut PgConnection) -> anyhow::Result<()> {
            conn.batch_execute(&format!(r#"SET search_path TO "{}";"#, self.schema))?;
            Ok(())
        }

        fn pool(&self) -> anyhow::Result<Pool<ConnectionManager<PgConnection>>> {
            let separator = if self.database_url.contains('?') {
                "&"
            } else {
                "?"
            };
            let database_url = format!(
                "{}{}options=-csearch_path%3D{}",
                self.database_url, separator, self.schema
            );
            let manager = ConnectionManager::<PgConnection>::new(database_url);
            Ok(Pool::builder().max_size(4).build(manager)?)
        }
    }

    impl Drop for TestSchema {
        fn drop(&mut self) {
            if let Ok(mut conn) = PgConnection::establish(&self.database_url) {
                let _ = conn.batch_execute(&format!(
                    r#"DROP SCHEMA IF EXISTS "{}" CASCADE;"#,
                    self.schema
                ));
            }
        }
    }

    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    #[derive(QueryableByName)]
    struct ColumnName {
        #[diesel(sql_type = Text)]
        column_name: String,
    }

    #[derive(QueryableByName)]
    struct ColumnDataType {
        #[diesel(sql_type = Text)]
        data_type: String,
    }

    #[derive(QueryableByName)]
    struct ColumnNullable {
        #[diesel(sql_type = Text)]
        is_nullable: String,
    }

    #[test]
    fn embedded_migrations_apply_and_revert() -> anyhow::Result<()> {
        let Some((schema, mut conn)) = TestSchema::new("migrations")? else {
            return Ok(());
        };

        conn.run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("failed to run migrations: {e}"))?;

        let invoice_table_count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'invoice'",
        )
        .get_result(&mut conn)?;
        assert_eq!(invoice_table_count.count, 1);

        let arkade_invoice_table_count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'arkade_invoice'",
        )
        .get_result(&mut conn)?;
        assert_eq!(arkade_invoice_table_count.count, 1);

        let arkade_swap_storage_table_count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'arkade_swap_storage'",
        )
        .get_result(&mut conn)?;
        assert_eq!(arkade_swap_storage_table_count.count, 1);

        let custom_addresses_table_count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'custom_addresses'",
        )
        .get_result(&mut conn)?;
        assert_eq!(custom_addresses_table_count.count, 1);

        let custom_address_invoice_table_count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'custom_address_invoice'",
        )
        .get_result(&mut conn)?;
        assert_eq!(custom_address_invoice_table_count.count, 1);

        let users_table_count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'users'",
        )
        .get_result(&mut conn)?;
        assert_eq!(users_table_count.count, 0);

        let columns: Vec<ColumnName> = diesel::sql_query(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'invoice'",
        )
        .load(&mut conn)?;
        let columns = columns
            .into_iter()
            .map(|column| column.column_name)
            .collect::<Vec<_>>();

        for expected in [
            "ark_address",
            "payment_hash",
            "created_at",
            "expires_at",
            "settled_at",
        ] {
            assert!(
                columns.iter().any(|column| column == expected),
                "missing invoice.{expected} column"
            );
        }

        let custom_invoice_columns: Vec<ColumnName> = diesel::sql_query(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'custom_address_invoice'",
        )
        .load(&mut conn)?;
        let custom_invoice_columns = custom_invoice_columns
            .into_iter()
            .map(|column| column.column_name)
            .collect::<Vec<_>>();

        for expected in [
            "name",
            "ark_address",
            "fee_receive_address",
            "payment_hash",
            "preimage",
            "ark_payment_reference",
            "settled_at",
        ] {
            assert!(
                custom_invoice_columns
                    .iter()
                    .any(|column| column == expected),
                "missing custom_address_invoice.{expected} column"
            );
        }

        let custom_invoice_ark_payment_reference_type: ColumnDataType = diesel::sql_query(
            "SELECT data_type FROM information_schema.columns \
             WHERE table_schema = current_schema() \
               AND table_name = 'custom_address_invoice' \
               AND column_name = 'ark_payment_reference'",
        )
        .get_result(&mut conn)?;
        assert_eq!(custom_invoice_ark_payment_reference_type.data_type, "text");

        let custom_invoice_expires_at_nullable: ColumnNullable = diesel::sql_query(
            "SELECT is_nullable FROM information_schema.columns \
             WHERE table_schema = current_schema() \
               AND table_name = 'custom_address_invoice' \
               AND column_name = 'expires_at'",
        )
        .get_result(&mut conn)?;
        assert_eq!(custom_invoice_expires_at_nullable.is_nullable, "NO");

        let arkade_columns: Vec<ColumnName> = diesel::sql_query(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'arkade_invoice'",
        )
        .load(&mut conn)?;
        let arkade_columns = arkade_columns
            .into_iter()
            .map(|column| column.column_name)
            .collect::<Vec<_>>();

        for expected in [
            "recipient_address",
            "payment_hash",
            "swap_id",
            "created_at",
            "expires_at",
            "settled_at",
        ] {
            assert!(
                arkade_columns.iter().any(|column| column == expected),
                "missing arkade_invoice.{expected} column"
            );
        }

        conn.revert_all_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("failed to revert migrations: {e}"))?;
        schema.set_search_path(&mut conn)?;

        let invoice_table_count: Count = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = 'invoice'",
        )
        .get_result(&mut conn)?;
        assert_eq!(invoice_table_count.count, 0);

        Ok(())
    }

    #[test]
    fn invoice_model_uses_claim_columns() -> anyhow::Result<()> {
        let Some((_schema, mut conn)) = TestSchema::new("invoice_model")? else {
            return Ok(());
        };

        conn.run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("failed to run migrations: {e}"))?;

        let expired_invoice = NewInvoice {
            ark_address: "ark-test-address".to_string(),
            bolt11: "lnbc1expired".to_string(),
            amount_msats: 1_000,
            payment_hash: Some("00".repeat(32)),
            preimage: String::new(),
            lnurlp_comment: None,
            state: InvoiceState::Pending as i32,
            expires_at: Some((chrono::Utc::now() - ChronoDuration::minutes(1)).naive_utc()),
        }
        .insert(&mut conn)?;

        let active_invoice = NewInvoice {
            ark_address: "ark-test-address".to_string(),
            bolt11: "lnbc1active".to_string(),
            amount_msats: 2_000,
            payment_hash: Some("11".repeat(32)),
            preimage: String::new(),
            lnurlp_comment: None,
            state: InvoiceState::Pending as i32,
            expires_at: Some((chrono::Utc::now() + ChronoDuration::minutes(5)).naive_utc()),
        }
        .insert(&mut conn)?;

        assert!(expired_invoice.created_at <= chrono::Utc::now().naive_utc());
        assert_eq!(Invoice::cancel_expired_pending(&mut conn)?, 1);

        let expired_invoice = Invoice::get_by_id(&mut conn, expired_invoice.id)?.unwrap();
        assert_eq!(expired_invoice.state, InvoiceState::Cancelled as i32);

        assert!(active_invoice.mark_settled(&mut conn, "22".repeat(32))?);
        let active_invoice = Invoice::get_by_id(&mut conn, active_invoice.id)?.unwrap();
        assert_eq!(active_invoice.state, InvoiceState::Settled as i32);
        assert_eq!(active_invoice.preimage, "22".repeat(32));
        assert!(active_invoice.settled_at.is_some());

        Ok(())
    }

    #[test]
    fn custom_address_invoice_model_releases_cancelled_names() -> anyhow::Result<()> {
        let Some((_schema, mut conn)) = TestSchema::new("custom_invoice_expiry")? else {
            return Ok(());
        };

        conn.run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("failed to run migrations: {e}"))?;

        let expired_invoice = NewCustomAddressInvoice {
            name: "alice".to_string(),
            ark_address: "ark-test-address".to_string(),
            auth_message: "message".to_string(),
            signature: "signature".to_string(),
            fee_receive_address: "ark-fee-address-1".to_string(),
            bolt11: "lnbc1expired".to_string(),
            amount_msats: 1_000,
            payment_hash: Some("33".repeat(32)),
            preimage: String::new(),
            ark_payment_reference: None,
            state: InvoiceState::Pending as i32,
            expires_at: (chrono::Utc::now() - ChronoDuration::minutes(1)).naive_utc(),
        }
        .insert(&mut conn)?;

        let active_invoice = NewCustomAddressInvoice {
            name: "bob".to_string(),
            ark_address: "ark-test-address".to_string(),
            auth_message: "message".to_string(),
            signature: "signature".to_string(),
            fee_receive_address: "ark-fee-address-2".to_string(),
            bolt11: "lnbc1active".to_string(),
            amount_msats: 1_000,
            payment_hash: Some("44".repeat(32)),
            preimage: String::new(),
            ark_payment_reference: None,
            state: InvoiceState::Pending as i32,
            expires_at: (chrono::Utc::now() + ChronoDuration::minutes(5)).naive_utc(),
        }
        .insert(&mut conn)?;

        assert!(CustomAddressInvoice::pending_name_exists(
            &mut conn, "alice"
        )?);

        assert!(expired_invoice.mark_cancelled(&mut conn)?);
        assert!(!CustomAddressInvoice::pending_name_exists(
            &mut conn, "alice"
        )?);
        assert!(CustomAddressInvoice::pending_name_exists(&mut conn, "bob")?);

        let expired_invoice =
            CustomAddressInvoice::get_by_id(&mut conn, expired_invoice.id)?.unwrap();
        assert_eq!(expired_invoice.state, InvoiceState::Cancelled as i32);
        let active_invoice =
            CustomAddressInvoice::get_by_id(&mut conn, active_invoice.id)?.unwrap();
        assert_eq!(active_invoice.state, InvoiceState::Pending as i32);

        Ok(())
    }

    #[tokio::test]
    async fn custom_address_sign_message_flow_creates_invoice() -> anyhow::Result<()> {
        let Some((schema, _conn)) = TestSchema::new("custom_address_sign_message")? else {
            return Ok(());
        };

        let db_pool = schema.pool()?;
        run_migrations(&db_pool)?;

        let (ark_address, vtxo_key) = test_bark_address()?;
        let ark_address = ark_address.to_string();
        let fee_receive_address = "ark-generated-fee-address".to_string();
        let invoice = test_invoice(50_000);
        let barkd = MockBarkd::start(MockBarkdState {
            expected_fee_receive_address: fee_receive_address.to_string(),
            expected_amount_sat: 50,
            invoice: invoice.to_string(),
            invoice_description: None,
            ark_payment_received: false,
            receive_status: None,
            receive_error: false,
            list_error: false,
        })
        .await?;

        let state = State {
            db_pool: db_pool.clone(),
            keys: Keys::generate(),
            barkd: Arc::new(BarkdClient::new(barkd.base_url(), None)?),
            arkade: None,
            domain: "example.com".to_string(),
            network: bitcoin::Network::Bitcoin,
            min_sendable: 1_000,
            max_sendable: 1_000_000,
            custom_address_fee_sats: 50,
        };
        let app = Router::new()
            .route(
                "/custom-addresses/auth-message",
                get(custom_address_auth_message),
            )
            .route("/custom-addresses", post(create_custom_address_invoice))
            .route("/custom-addresses/{id}", get(get_custom_address_invoice))
            .layer(Extension(state));
        let app = TestServer::start(app).await?;
        let client = reqwest::Client::new();

        let auth: Value = client
            .get(format!(
                "{}/custom-addresses/auth-message?name=Alice&arkAddress={}",
                app.base_url(),
                ark_address
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let auth_message = auth["message"].as_str().unwrap().to_string();
        assert_eq!(auth["name"], "alice");
        assert_eq!(auth["arkAddress"], ark_address);
        assert_eq!(
            auth_message,
            format!("arkzap.me custom address\nname: alice\ndomain: example.com\nark_address: {ark_address}")
        );
        assert_eq!(auth["signing"]["minimumBarkVersion"], "0.6.2");
        assert_eq!(
            auth["signing"]["cliCommand"],
            json!(["bark", "message", "sign", auth_message, ark_address])
        );
        assert_eq!(auth["signing"]["barkdRequest"]["method"], "POST");
        assert_eq!(
            auth["signing"]["barkdRequest"]["path"],
            "/api/v1/message/sign"
        );
        assert_eq!(
            auth["signing"]["barkdRequest"]["body"],
            json!({
                "message": auth_message,
                "address": ark_address,
            })
        );
        let signature = ark::message::sign(&vtxo_key, auth_message.as_bytes()).to_string();

        let created_response = client
            .post(format!("{}/custom-addresses", app.base_url()))
            .json(&json!({
                "name": "Alice",
                "arkAddress": ark_address,
                "signature": signature,
            }))
            .send()
            .await?;
        let status = created_response.status();
        let created: Value = created_response.json().await?;

        assert_eq!(status, reqwest::StatusCode::OK, "{created}");

        assert_eq!(created["status"], "OK");
        assert_eq!(created["customAddress"], "alice@example.com");
        assert_eq!(created["invoice"]["name"], "alice");
        assert_eq!(created["invoice"]["arkAddress"], fee_receive_address);
        assert_eq!(created["invoice"]["invoice"], invoice.to_string());
        assert_eq!(created["invoice"]["state"], "pending");
        assert_eq!(created["invoice"]["active"], false);

        let mut conn = db_pool.get()?;
        let invoices = CustomAddressInvoice::get_by_state(&mut conn, InvoiceState::Pending as i32)?;
        assert_eq!(invoices.len(), 1);
        assert_eq!(invoices[0].name, "alice");
        assert_eq!(invoices[0].ark_address, ark_address);
        assert_eq!(invoices[0].auth_message, auth_message);
        assert_eq!(invoices[0].signature, signature);
        assert_eq!(invoices[0].fee_receive_address, fee_receive_address);
        assert_eq!(invoices[0].ark_payment_reference, None);
        assert_eq!(invoices[0].amount_msats, 50_000);
        assert_eq!(
            invoices[0].payment_hash.as_deref(),
            Some(invoice.payment_hash().to_string().as_str())
        );

        {
            let mock_state = barkd.state.lock().unwrap();
            assert_eq!(
                mock_state.invoice_description.as_deref(),
                Some("arkzap.me custom address alice")
            );
        }

        {
            let mut mock_state = barkd.state.lock().unwrap();
            mock_state.ark_payment_received = true;
        }

        let purchase_id = created["invoice"]["id"].as_i64().unwrap();
        let activated: Value = client
            .get(format!(
                "{}/custom-addresses/{}",
                app.base_url(),
                purchase_id
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        assert_eq!(activated["invoice"]["state"], "settled");
        assert_eq!(activated["invoice"]["active"], true);
        assert_eq!(activated["invoice"]["arkAddress"], fee_receive_address);

        let custom_address = CustomAddress::get_by_name(&mut conn, "alice")?.unwrap();
        assert_eq!(custom_address.ark_address, ark_address);
        let invoice = CustomAddressInvoice::get_by_id(&mut conn, purchase_id as i32)?.unwrap();
        assert_eq!(
            invoice.ark_payment_reference.as_deref(),
            Some("bark-movement:1")
        );
        assert_eq!(invoice.preimage, "");

        Ok(())
    }

    struct ExpiredBarkPayment {
        state: State,
        barkd: MockBarkd,
        invoice: Invoice,
        custom_invoice: CustomAddressInvoice,
        _schema: TestSchema,
    }

    impl ExpiredBarkPayment {
        async fn new() -> anyhow::Result<Option<Self>> {
            let Some((schema, _conn)) = TestSchema::new("bark_expiry")? else {
                return Ok(None);
            };
            let db_pool = schema.pool()?;
            run_migrations(&db_pool)?;
            let address = test_bark_address()?.0.to_string();
            let bolt11 = test_invoice_at(50_000, Duration::from_secs(1));
            assert!(bolt11.is_expired());
            let barkd = MockBarkd::start(MockBarkdState {
                expected_fee_receive_address: address.clone(),
                expected_amount_sat: 50,
                invoice: bolt11.to_string(),
                invoice_description: None,
                ark_payment_received: false,
                receive_status: None,
                receive_error: false,
                list_error: false,
            })
            .await?;
            let state = State {
                db_pool,
                keys: Keys::generate(),
                barkd: Arc::new(BarkdClient::new(barkd.base_url(), None)?),
                arkade: None,
                domain: "example.com".to_string(),
                network: bitcoin::Network::Bitcoin,
                min_sendable: 1_000,
                max_sendable: 1_000_000,
                custom_address_fee_sats: 50,
            };
            let (invoice, custom_invoice) = {
                let mut conn = state.db_pool.get()?;
                let expires_at = (chrono::Utc::now() - ChronoDuration::minutes(1)).naive_utc();
                let invoice = NewInvoice {
                    ark_address: address.clone(),
                    bolt11: bolt11.to_string(),
                    amount_msats: 50_000,
                    payment_hash: Some(bolt11.payment_hash().to_string()),
                    preimage: String::new(),
                    lnurlp_comment: None,
                    state: InvoiceState::Pending as i32,
                    expires_at: Some(expires_at),
                }
                .insert(&mut conn)?;
                let custom_invoice = NewCustomAddressInvoice {
                    name: "alice".to_string(),
                    ark_address: address.clone(),
                    auth_message: "message".to_string(),
                    signature: "signature".to_string(),
                    fee_receive_address: address,
                    bolt11: bolt11.to_string(),
                    amount_msats: 50_000,
                    payment_hash: Some(bolt11.payment_hash().to_string()),
                    preimage: String::new(),
                    ark_payment_reference: None,
                    state: InvoiceState::Pending as i32,
                    expires_at,
                }
                .insert(&mut conn)?;
                (invoice, custom_invoice)
            };
            Ok(Some(Self {
                state,
                barkd,
                invoice,
                custom_invoice,
                _schema: schema,
            }))
        }

        fn set_receive(&self, phase: &str, with_preimage: bool) {
            self.barkd.state.lock().unwrap().receive_status = Some(json!({
                "payment_hash": self.invoice.payment_hash,
                "state": phase,
                "invoice": self.invoice.bolt11,
                "payment_preimage": with_preimage.then(|| "08".repeat(32)),
                "amount_sat": 50,
                "htlc_vtxo_ids": [],
                "settled_at": null,
            }));
        }

        fn assert_state(&self, expected: InvoiceState) -> anyhow::Result<()> {
            let mut conn = self.state.db_pool.get()?;
            let invoice = Invoice::get_by_id(&mut conn, self.invoice.id)?.unwrap();
            let custom =
                CustomAddressInvoice::get_by_id(&mut conn, self.custom_invoice.id)?.unwrap();
            assert_eq!(invoice.state, expected as i32);
            assert_eq!(custom.state, expected as i32);
            assert_eq!(
                CustomAddress::name_exists(&mut conn, "alice")?,
                expected == InvoiceState::Settled
            );
            assert_eq!(
                CustomAddressInvoice::pending_name_exists(&mut conn, "alice")?,
                expected == InvoiceState::Pending
            );
            if expected == InvoiceState::Settled {
                assert_eq!(invoice.preimage, "08".repeat(32));
                assert_eq!(custom.preimage, "08".repeat(32));
            }
            Ok(())
        }

        async fn refresh_routes(&self) -> anyhow::Result<()> {
            let result = verify(
                axum::extract::Path((
                    self.invoice.ark_address.clone(),
                    invoice_payment_hash(&self.invoice),
                )),
                Extension(self.state.clone()),
            )
            .await;
            let _ = result.map_err(|e| anyhow::anyhow!("verify failed: {e:?}"))?;
            let _ = get_custom_address_invoice(
                axum::extract::Path(self.custom_invoice.id),
                Extension(self.state.clone()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("custom invoice refresh failed: {e:?}"))?;
            Ok(())
        }

        async fn request_same_name(&self) -> Result<Json<Value>, (http::StatusCode, Json<Value>)> {
            let (address, key) = test_bark_address().unwrap();
            let message = format!("arkzap.me custom address\nname: alice\ndomain: example.com\nark_address: {address}");
            create_custom_address_invoice(
                Extension(self.state.clone()),
                Json(CreateCustomAddressInvoiceRequest {
                    name: "alice".to_string(),
                    ark_address: address.to_string(),
                    signature: ark::message::sign(&key, message.as_bytes()).to_string(),
                }),
            )
            .await
        }
    }

    #[tokio::test]
    async fn accepted_bark_payments_settle_after_invoice_expiry() -> anyhow::Result<()> {
        for list_error in [false, true] {
            for final_phase in ["preimage-revealed", "delivering", "settled"] {
                let Some(payment) = ExpiredBarkPayment::new().await? else {
                    return Ok(());
                };
                payment.barkd.state.lock().unwrap().list_error = list_error;
                payment.set_receive("htlcs-ready", true);

                // Both the batch poller and its per-invoice fallback must retain accepted payments.
                claim_paid_bark_invoices_once(&payment.state).await?;
                payment.assert_state(InvoiceState::Pending)?;
                payment.refresh_routes().await?;
                payment.assert_state(InvoiceState::Pending)?;

                // A competing name request must not expire the accepted fee payment.
                let (_, Json(error)) = payment.request_same_name().await.unwrap_err();
                assert_eq!(
                    error["reason"],
                    "Custom address already has a pending invoice"
                );
                payment.assert_state(InvoiceState::Pending)?;

                payment.set_receive(final_phase, true);
                // Exercise settlement through each entry point, then check idempotent polling.
                if list_error {
                    payment.refresh_routes().await?;
                }
                claim_paid_bark_invoices_once(&payment.state).await?;
                payment.refresh_routes().await?;
                payment.assert_state(InvoiceState::Settled)?;
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn bark_expiry_requires_an_unpaid_or_missing_receive() -> anyhow::Result<()> {
        for phase in [
            None,
            Some("awaiting-payment"),
            Some("unknown-state"),
            Some("preimage-revealed"),
            Some("delivering"),
            Some("settled"),
        ] {
            let Some(payment) = ExpiredBarkPayment::new().await? else {
                return Ok(());
            };
            if let Some(phase) = phase {
                // An awaiting payment can expose a known preimage without having revealed it.
                payment.set_receive(phase, phase == "awaiting-payment");
            }
            let expected = if phase.is_none() || phase == Some("awaiting-payment") {
                InvoiceState::Cancelled
            } else {
                InvoiceState::Pending
            };
            payment.refresh_routes().await?;
            payment.assert_state(expected)?;
            claim_paid_bark_invoices_once(&payment.state).await?;
            payment.assert_state(expected)?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn bark_status_errors_keep_expired_payments_and_names_pending() -> anyhow::Result<()> {
        let Some(payment) = ExpiredBarkPayment::new().await? else {
            return Ok(());
        };
        {
            let mut mock = payment.barkd.state.lock().unwrap();
            mock.receive_error = true;
            mock.list_error = true;
        }
        claim_paid_bark_invoices_once(&payment.state).await?;
        payment.assert_state(InvoiceState::Pending)?;
        assert!(payment.refresh_routes().await.is_err());
        assert!(get_custom_address_invoice(
            axum::extract::Path(payment.custom_invoice.id),
            Extension(payment.state.clone())
        )
        .await
        .is_err());
        assert!(payment.request_same_name().await.is_err());
        payment.assert_state(InvoiceState::Pending)?;
        Ok(())
    }

    #[tokio::test]
    async fn expired_unpaid_custom_name_can_be_reserved_again() -> anyhow::Result<()> {
        let Some(payment) = ExpiredBarkPayment::new().await? else {
            return Ok(());
        };
        payment.set_receive("awaiting-payment", true);
        let key = BitcoinSecretKey::from_slice(&[42; 32])?;
        let fresh_invoice = InvoiceBuilder::new(Currency::Bitcoin)
            .amount_milli_satoshis(50_000)
            .description("replacement custom address fee".to_string())
            .payment_hash(sha256::Hash::hash(&[9; 32]))
            .payment_secret(PaymentSecret([9; 32]))
            .current_timestamp()
            .min_final_cltv_expiry_delta(144)
            .build_signed(|hash| BitcoinSecp256k1::new().sign_ecdsa_recoverable(hash, &key))?;
        payment.barkd.state.lock().unwrap().invoice = fresh_invoice.to_string();

        let Json(created) = payment
            .request_same_name()
            .await
            .map_err(|e| anyhow::anyhow!("replacement reservation failed: {e:?}"))?;
        assert_eq!(created["invoice"]["state"], "pending");
        assert_ne!(created["invoice"]["id"], payment.custom_invoice.id);
        let mut conn = payment.state.db_pool.get()?;
        let old = CustomAddressInvoice::get_by_id(&mut conn, payment.custom_invoice.id)?.unwrap();
        assert_eq!(old.state, InvoiceState::Cancelled as i32);
        let replacement = CustomAddressInvoice::get_pending_by_name(&mut conn, "alice")?.unwrap();
        assert_eq!(replacement.bolt11, fresh_invoice.to_string());
        Ok(())
    }

    #[tokio::test]
    async fn expired_custom_name_checks_ark_payment_before_cancelling() -> anyhow::Result<()> {
        let Some(payment) = ExpiredBarkPayment::new().await? else {
            return Ok(());
        };
        payment.barkd.state.lock().unwrap().ark_payment_received = true;
        // The same-name request must reconcile the old fee before releasing the name.
        let (_, Json(error)) = payment.request_same_name().await.unwrap_err();
        assert_eq!(error["reason"], "Custom address is already taken");
        let mut conn = payment.state.db_pool.get()?;
        let custom =
            CustomAddressInvoice::get_by_id(&mut conn, payment.custom_invoice.id)?.unwrap();
        assert_eq!(custom.state, InvoiceState::Settled as i32);
        assert_eq!(
            custom.ark_payment_reference.as_deref(),
            Some("bark-movement:1")
        );
        assert!(CustomAddress::name_exists(&mut conn, "alice")?);
        Ok(())
    }

    struct MockBarkdState {
        expected_fee_receive_address: String,
        expected_amount_sat: u64,
        invoice: String,
        invoice_description: Option<String>,
        ark_payment_received: bool,
        receive_status: Option<Value>,
        receive_error: bool,
        list_error: bool,
    }

    struct MockBarkd {
        addr: SocketAddr,
        state: Arc<Mutex<MockBarkdState>>,
        shutdown: Option<oneshot::Sender<()>>,
    }

    impl MockBarkd {
        async fn start(state: MockBarkdState) -> anyhow::Result<Self> {
            let state = Arc::new(Mutex::new(state));
            let app = Router::new()
                .route("/api/v1/wallet/addresses/next", post(mock_new_address))
                .route(
                    "/api/v1/lightning/receives/invoice/for-address",
                    post(mock_invoice_for_address),
                )
                .route(
                    "/api/v1/lightning/receives/{identifier}",
                    get(mock_receive_status),
                )
                .route("/api/v1/lightning/receives", get(mock_list_receives))
                .route("/api/v1/history", get(mock_history))
                .with_state(state.clone());
            let (addr, shutdown) = spawn_router(app).await?;

            Ok(Self {
                addr,
                state,
                shutdown: Some(shutdown),
            })
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }
    }

    impl Drop for MockBarkd {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
        }
    }

    struct TestServer {
        addr: SocketAddr,
        shutdown: Option<oneshot::Sender<()>>,
    }

    impl TestServer {
        async fn start(app: Router) -> anyhow::Result<Self> {
            let (addr, shutdown) = spawn_router(app).await?;
            Ok(Self {
                addr,
                shutdown: Some(shutdown),
            })
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
        }
    }

    async fn spawn_router(app: Router) -> anyhow::Result<(SocketAddr, oneshot::Sender<()>)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        Ok((addr, shutdown_tx))
    }

    async fn mock_new_address(
        axum::extract::State(state): axum::extract::State<Arc<Mutex<MockBarkdState>>>,
    ) -> Json<Value> {
        let state = state.lock().unwrap();
        Json(json!({ "address": state.expected_fee_receive_address }))
    }

    async fn mock_invoice_for_address(
        axum::extract::State(state): axum::extract::State<Arc<Mutex<MockBarkdState>>>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let mut state = state.lock().unwrap();
        assert_eq!(body["amount_sat"], state.expected_amount_sat);
        assert_eq!(body["address"], state.expected_fee_receive_address);
        state.invoice_description = body["description"].as_str().map(str::to_string);
        Json(json!({ "invoice": state.invoice }))
    }

    async fn mock_receive_status(
        axum::extract::State(state): axum::extract::State<Arc<Mutex<MockBarkdState>>>,
        axum::extract::Path(_identifier): axum::extract::Path<String>,
    ) -> Result<Json<Value>, axum::http::StatusCode> {
        let state = state.lock().unwrap();
        if state.receive_error {
            return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
        }
        state
            .receive_status
            .clone()
            .map(Json)
            .ok_or(axum::http::StatusCode::NOT_FOUND)
    }

    async fn mock_list_receives(
        axum::extract::State(state): axum::extract::State<Arc<Mutex<MockBarkdState>>>,
    ) -> Result<Json<Value>, axum::http::StatusCode> {
        let state = state.lock().unwrap();
        if state.list_error {
            return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
        }
        Ok(Json(json!(state.receive_status.iter().collect::<Vec<_>>())))
    }

    async fn mock_history(
        axum::extract::State(state): axum::extract::State<Arc<Mutex<MockBarkdState>>>,
    ) -> Json<Value> {
        let state = state.lock().unwrap();
        if !state.ark_payment_received {
            return Json(json!([]));
        }

        Json(json!([{
            "id": 1,
            "status": "successful",
            "subsystem": {
                "name": "arkoor",
                "kind": "receive"
            },
            "intended_balance_sat": state.expected_amount_sat,
            "effective_balance_sat": state.expected_amount_sat,
            "offchain_fee_sat": 0,
            "sent_to": [],
            "received_on": [{
                "destination": {
                    "type": "ark",
                    "value": state.expected_fee_receive_address
                },
                "amount_sat": state.expected_amount_sat
            }],
            "input_vtxos": [],
            "output_vtxos": [],
            "exited_vtxos": [],
            "time": {
                "created_at": "2026-06-17T00:00:00-05:00",
                "updated_at": "2026-06-17T00:00:00-05:00",
                "completed_at": "2026-06-17T00:00:00-05:00"
            }
        }]))
    }

    fn test_invoice(amount_msats: u64) -> lightning_invoice::Bolt11Invoice {
        test_invoice_at(
            amount_msats,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap(),
        )
    }

    fn test_invoice_at(amount_msats: u64, timestamp: Duration) -> lightning_invoice::Bolt11Invoice {
        let private_key = BitcoinSecretKey::from_slice(&[42; 32]).unwrap();
        let payment_hash = sha256::Hash::hash(&[8; 32]);
        let payment_secret = PaymentSecret([9; 32]);

        InvoiceBuilder::new(Currency::Bitcoin)
            .amount_milli_satoshis(amount_msats)
            .description("custom address fee".to_string())
            .payment_hash(payment_hash)
            .payment_secret(payment_secret)
            .duration_since_epoch(timestamp)
            .min_final_cltv_expiry_delta(144)
            .build_signed(|hash| BitcoinSecp256k1::new().sign_ecdsa_recoverable(hash, &private_key))
            .unwrap()
    }

    fn test_bark_address() -> anyhow::Result<(ark::Address, Keypair)> {
        let secp = Secp256k1::new();
        let server_key = test_keypair(&secp, 1)?;
        let vtxo_key = test_keypair(&secp, 2)?;
        let server_mailbox_key = test_keypair(&secp, 3)?;
        let bark_mailbox_key = test_keypair(&secp, 4)?;
        let mailbox = MailboxIdentifier::from_pubkey(bark_mailbox_key.public_key());

        let address = ark::Address::builder()
            .server_pubkey(server_key.public_key())
            .pubkey_policy(vtxo_key.public_key())
            .mailbox(server_mailbox_key.public_key(), mailbox, &vtxo_key)?
            .into_address()?;

        Ok((address, vtxo_key))
    }

    fn test_keypair<C: ark::bitcoin::secp256k1::Signing>(
        secp: &Secp256k1<C>,
        byte: u8,
    ) -> anyhow::Result<Keypair> {
        let secret_key = SecretKey::from_slice(&[byte; 32])?;
        Ok(Keypair::from_secret_key(secp, &secret_key))
    }
}
