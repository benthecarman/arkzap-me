use crate::models::arkade_invoice::{ArkadeInvoice, NewArkadeInvoice};
use crate::models::arkade_zap::ArkadeZap;
use crate::models::custom_address::{CustomAddress, CustomAddressInvoice, NewCustomAddressInvoice};
use crate::models::invoice::{Invoice, InvoiceState, NewInvoice};
use crate::models::zap::Zap;
use crate::State;
use anyhow::anyhow;
use axum::extract::{Path, Query};
use axum::http::{StatusCode, Uri};
use axum::response::Html;
use axum::{Extension, Json};
use bitcoin::Network;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDateTime, Utc};
use diesel::Connection;
use lightning_invoice::Bolt11Invoice;
use lnurl::pay::PayResponse;
use lnurl::Tag;
use log::{error, info, warn};
use nostr::{Event, JsonUtil};
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::fmt::Display;
use std::str::FromStr;
use std::time::SystemTime;

const MAX_NOSTR_PARAM_LEN: usize = 16 * 1024;
const ARKADE_MIN_SENDABLE_MSATS: u64 = 333_000;
const MAX_CUSTOM_SIGNATURE_LEN: usize = 128;
const CUSTOM_ADDRESS_INVOICE_EXPIRY: ChronoDuration = ChronoDuration::hours(1);

pub async fn root() -> Html<&'static str> {
    Html(concat!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>arkzap.me</title>
  <style>
    :root {
      color-scheme: light dark;
      --bg: #fafafa;
      --text: #151515;
      --muted: #555555;
      --line: #d9d9d9;
      --panel: #ffffff;
      --accent: #0f766e;
      --accent-2: #c2410c;
      --code: #252422;
    }

    * {
      box-sizing: border-box;
    }

    body {
      margin: 0;
      min-height: 100vh;
      background: var(--bg);
      color: var(--text);
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      letter-spacing: 0;
    }

    main {
      width: min(1080px, calc(100% - 32px));
      margin: 0 auto;
      padding: 56px 0 48px;
    }

    .hero {
      display: grid;
      grid-template-columns: minmax(0, 1.15fr) minmax(280px, 0.85fr);
      gap: 36px;
      align-items: center;
      padding-bottom: 48px;
      border-bottom: 1px solid var(--line);
    }

    h1 {
      margin: 0;
      font-size: 84px;
      line-height: 0.95;
      font-weight: 800;
    }

    .lede {
      max-width: 680px;
      margin: 22px 0 0;
      color: var(--muted);
      font-size: 20px;
      line-height: 1.55;
    }

    .address {
      display: inline-flex;
      align-items: center;
      max-width: 100%;
      margin-top: 26px;
      padding: 10px 12px;
      border: 1px solid var(--line);
      background: var(--panel);
      color: var(--code);
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 15px;
      overflow-wrap: anywhere;
    }

    .mark {
      position: relative;
      min-height: 320px;
      border: 1px solid var(--line);
      background:
        radial-gradient(circle at 28% 30%, rgba(15, 118, 110, 0.26), transparent 26%),
        radial-gradient(circle at 68% 68%, rgba(194, 65, 12, 0.22), transparent 28%),
        linear-gradient(135deg, #ffffff, #ededed);
      overflow: hidden;
    }

    .mark::before,
    .mark::after {
      content: "";
      position: absolute;
      inset: 58px;
      border: 2px solid rgba(21, 21, 21, 0.7);
      transform: rotate(45deg);
    }

    .mark::after {
      inset: 104px;
      border-color: var(--accent);
      transform: rotate(45deg) translate(22px, -22px);
    }

    .bolt {
      position: absolute;
      left: 50%;
      top: 50%;
      width: 76px;
      height: 128px;
      transform: translate(-50%, -50%) skewX(-12deg);
      background: var(--accent-2);
      clip-path: polygon(48% 0, 94% 0, 62% 43%, 100% 43%, 34% 100%, 48% 57%, 0 57%);
    }

    section {
      padding: 36px 0 0;
    }

    h2 {
      margin: 0 0 14px;
      font-size: 24px;
      line-height: 1.2;
    }

    p {
      color: var(--muted);
      line-height: 1.65;
    }

    .grid {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 16px;
    }

    .item {
      min-height: 132px;
      padding: 18px;
      border: 1px solid var(--line);
      background: var(--panel);
    }

    .item strong {
      display: block;
      margin-bottom: 8px;
      font-size: 17px;
    }

    .item span {
      color: var(--muted);
      line-height: 1.5;
    }

    .endpoints {
      display: grid;
      gap: 10px;
      margin: 0;
      padding: 0;
      list-style: none;
    }

    .endpoints li {
      display: flex;
      gap: 14px;
      align-items: baseline;
      padding: 12px 0;
      border-bottom: 1px solid var(--line);
    }

    code {
      color: var(--code);
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      overflow-wrap: anywhere;
    }

    .method {
      flex: 0 0 auto;
      color: var(--accent);
      font-weight: 800;
      font-size: 13px;
    }

    footer {
      padding-top: 36px;
      color: var(--muted);
      font-size: 14px;
    }

    @media (max-width: 760px) {
      main {
        width: min(100% - 24px, 1080px);
        padding-top: 28px;
      }

      .hero,
      .grid {
        grid-template-columns: 1fr;
      }

      h1 {
        font-size: 48px;
      }

      .mark {
        min-height: 220px;
      }

      .endpoints li {
        display: grid;
        gap: 5px;
      }
    }
  </style>
</head>
<body>
  <main>
    <div class="hero">
      <div>
        <h1>arkzap.me</h1>
        <p class="lede">LNURL-pay infrastructure for sending Lightning zaps to Bark and custom Bark-backed Lightning addresses.</p>
        <div class="address">address@arkzap.me</div>
      </div>
      <div class="mark" aria-hidden="true"><div class="bolt"></div></div>
    </div>

    <section>
      <h2>What It Does</h2>
      <div class="grid">
        <div class="item"><strong>LNURL-pay</strong><span>Publishes pay metadata for Bark, Arkade, and paid custom names.</span></div>
        <div class="item"><strong>Nostr zaps</strong><span>Accepts zap request events and stores invoice verification data.</span></div>
        <div class="item"><strong>Settlement checks</strong><span>Exposes verification for pending and settled invoices.</span></div>
      </div>
    </section>

    <section>
      <h2>Public Endpoints</h2>
      <ul class="endpoints">
        <li><span class="method">GET</span><code>/.well-known/lnurlp/:address</code></li>
        <li><span class="method">GET</span><code>/get-invoice/:address?amount=1000</code></li>
        <li><span class="method">GET</span><code>/verify/:address/:payment_hash</code></li>
        <li><span class="method">GET</span><code>/custom-addresses/auth-message?name=alice&amp;arkAddress=ark...</code></li>
        <li><span class="method">POST</span><code>/custom-addresses</code></li>
        <li><span class="method">GET</span><code>/custom-addresses/:id</code></li>
        <li><span class="method">GET</span><code>/health-check</code></li>
      </ul>
    </section>

    <footer>arkzap.me runs arkzap-me v"#,
        env!("CARGO_PKG_VERSION"),
        r#".</footer>
  </main>
</body>
</html>"#
    ))
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LnurlCallbackParams {
    pub amount: Option<u64>, // User specified amount in MilliSatoshi
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub nostr: Option<String>, // Optional zap request
}

/// Creates a Lightning invoice and optionally stores zap request information.
///
/// This is the core implementation for generating invoices for LNURL-pay requests.
///
/// # Parameters
/// * `state` - Application state containing LND client and configuration
/// * `address` - Receive address that the invoice is for
/// * `amount_msats` - The invoice amount in millisatoshis
/// * `zap_request` - Optional Nostr zap request event
///
/// # Returns
/// A BOLT11 invoice if successful, or an error
pub(crate) async fn get_invoice_impl(
    state: &State,
    address: &str,
    params: LnurlCallbackParams,
) -> anyhow::Result<Bolt11Invoice> {
    validate_callback_params(&params)?;

    if params.amount.is_none() {
        return Err(anyhow!("Missing amount parameter"));
    }

    let resolved_address = resolve_receive_address(state, address)?;
    let invoice_identifier = resolved_address.identifier.clone();
    let address = resolved_address.address;

    let amount_msats = params.amount.unwrap();
    validate_amount_msats(
        amount_msats,
        address.min_sendable_msats(state.min_sendable),
        state.max_sendable,
    )?;

    let mut zap_request = None;
    let _invoice_description = match params.nostr.as_ref() {
        None => calc_metadata(&invoice_identifier, &state.domain),
        Some(str) => {
            let event = Event::from_json(str).map_err(|_| anyhow!("Invalid zap request"))?;
            if event.kind != nostr::Kind::ZapRequest {
                return Err(anyhow!("Invalid zap request"));
            }
            zap_request = Some(event);
            str.clone()
        }
    };

    let invoice = match address {
        ReceiveAddress::Bark(ark_address) => {
            let invoice = state
                .barkd
                .invoice_for_address(
                    amount_msats / 1_000,
                    ark_address.to_string(),
                    Some(_invoice_description),
                )
                .await?;
            PendingInvoice::Bark {
                invoice,
                address: ark_address.to_string(),
            }
        }
        ReceiveAddress::Arkade(arkade_address) => {
            let arkade = state
                .arkade
                .as_ref()
                .ok_or_else(|| anyhow!("Arkade support is disabled"))?;
            let result = arkade
                .invoice_for_address(
                    amount_msats / 1_000,
                    arkade_address,
                    Some(_invoice_description),
                )
                .await?;
            PendingInvoice::Arkade {
                invoice: result.invoice,
                address: arkade_address.to_string(),
                swap_id: result.swap_id,
            }
        }
    };

    if !invoice
        .bolt11()
        .amount_milli_satoshis()
        .is_some_and(|a| a == amount_msats)
    {
        return Err(anyhow!("Invoice amount mismatch"));
    }

    let payment_hash = invoice.bolt11().payment_hash().to_string();
    let expires_at = invoice_expires_at(invoice.bolt11());
    let invoice_to_return = invoice.bolt11().clone();

    let mut conn = state.db_pool.get()?;
    conn.transaction::<_, anyhow::Error, _>(|conn| {
        match invoice {
            PendingInvoice::Bark { invoice, address } => {
                let invoice = NewInvoice {
                    ark_address: address,
                    bolt11: invoice.to_string(),
                    amount_msats: amount_msats as i64,
                    payment_hash: Some(payment_hash),
                    preimage: String::new(),
                    lnurlp_comment: None,
                    state: InvoiceState::Pending as i32,
                    expires_at,
                };
                let inserted_invoice = invoice.insert(conn)?;

                if let Some(zap_request) = zap_request {
                    let zap = Zap {
                        id: inserted_invoice.id,
                        request: zap_request.as_json(),
                        event_id: None,
                    };
                    zap.insert(conn)?;
                }
            }
            PendingInvoice::Arkade {
                invoice,
                address,
                swap_id,
            } => {
                let invoice = NewArkadeInvoice {
                    recipient_address: address,
                    bolt11: invoice.to_string(),
                    amount_msats: amount_msats as i64,
                    payment_hash: Some(payment_hash),
                    preimage: String::new(),
                    swap_id,
                    lnurlp_comment: None,
                    state: InvoiceState::Pending as i32,
                    expires_at,
                };
                let inserted_invoice = invoice.insert(conn)?;

                if let Some(zap_request) = zap_request {
                    let zap = ArkadeZap {
                        id: inserted_invoice.id,
                        request: zap_request.as_json(),
                        event_id: None,
                    };
                    zap.insert(conn)?;
                }
            }
        }

        Ok(())
    })?;

    Ok(invoice_to_return)
}

enum PendingInvoice {
    Bark {
        invoice: Bolt11Invoice,
        address: String,
    },
    Arkade {
        invoice: Bolt11Invoice,
        address: String,
        swap_id: String,
    },
}

impl PendingInvoice {
    fn bolt11(&self) -> &Bolt11Invoice {
        match self {
            PendingInvoice::Bark { invoice, .. } | PendingInvoice::Arkade { invoice, .. } => {
                invoice
            }
        }
    }
}

fn invoice_expires_at(invoice: &Bolt11Invoice) -> Option<NaiveDateTime> {
    let expires_at = invoice.expires_at()?;
    let expires_at = SystemTime::UNIX_EPOCH.checked_add(expires_at)?;
    Some(DateTime::<Utc>::from(expires_at).naive_utc())
}

/// HTTP endpoint for generating Lightning invoices from a LNURL-pay request.
///
/// This route handles the callback phase of the LNURL-pay protocol.
///
/// # Parameters
/// * `ark_address` - Path parameter containing the receive address
/// * `params` - Query parameters including the amount and optional zap request
/// * `state` - Application state
///
/// # Returns
/// A JSON response with the invoice and verification URL, or an error response
pub async fn get_invoice(
    Path(ark_address): Path<String>,
    Query(params): Query<LnurlCallbackParams>,
    Extension(state): Extension<State>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let amount_msats = params.amount;

    match get_invoice_impl(&state, &ark_address, params).await {
        Ok(invoice) => {
            let payment_hash = invoice.payment_hash().to_string();
            let verify_url = format!(
                "https://{}/verify/{ark_address}/{payment_hash}",
                state.domain
            );
            Ok(Json(json!({
                "status": "OK",
                "pr": invoice,
                "verify": verify_url,
                "routes": [],
            })))
        }
        Err(e) => {
            if should_log_invoice_error(&e) {
                let address_kind = parse_receive_address(&ark_address)
                    .map(|address| address.kind())
                    .unwrap_or("invalid");
                error!(
                    "Error generating invoice for ark_address={ark_address} address_kind={address_kind} amount_msats={amount_msats:?}: {e:#}"
                );
            }
            Err(handle_anyhow_error(e))
        }
    }
}

pub fn calc_metadata(ark_address: &str, domain: &str) -> String {
    format!(
        "[[\"text/identifier\",\"{ark_address}@{domain}\"],[\"text/plain\",\"Sats for {ark_address}\"]]",
    )
}

fn validate_amount_msats(
    amount_msats: u64,
    min_sendable: u64,
    max_sendable: u64,
) -> anyhow::Result<()> {
    if amount_msats < min_sendable || amount_msats > max_sendable {
        return Err(anyhow!("Amount out of bounds"));
    }
    if amount_msats % 1_000 != 0 {
        return Err(anyhow!("Bark invoices must be denominated in whole sats"));
    }

    Ok(())
}

fn validate_callback_params(params: &LnurlCallbackParams) -> anyhow::Result<()> {
    if params
        .nostr
        .as_ref()
        .is_some_and(|nostr| nostr.len() > MAX_NOSTR_PARAM_LEN)
    {
        return Err(anyhow!("Nostr parameter is too large"));
    }

    Ok(())
}

fn should_log_invoice_error(err: &anyhow::Error) -> bool {
    !err.chain()
        .any(|cause| cause.to_string() == "Missing amount parameter")
}

struct ResolvedReceiveAddress {
    address: ReceiveAddress,
    identifier: String,
}

enum ReceiveAddress {
    Bark(ark::Address),
    Arkade(ark_core::ArkAddress),
}

impl ReceiveAddress {
    fn kind(&self) -> &'static str {
        match self {
            ReceiveAddress::Bark(_) => "bark",
            ReceiveAddress::Arkade(_) => "arkade",
        }
    }

    fn min_sendable_msats(&self, configured_min_sendable: u64) -> u64 {
        match self {
            ReceiveAddress::Bark(_) => configured_min_sendable,
            ReceiveAddress::Arkade(_) => configured_min_sendable.max(ARKADE_MIN_SENDABLE_MSATS),
        }
    }

    fn validate_network(&self, network: Network) -> anyhow::Result<()> {
        let expects_test_address = network != Network::Bitcoin;
        let is_test_address = match self {
            ReceiveAddress::Bark(address) => address.is_testnet(),
            ReceiveAddress::Arkade(address) => address.to_string().starts_with("tark"),
        };

        if is_test_address != expects_test_address {
            return Err(anyhow!("Address is not valid for configured network"));
        }

        Ok(())
    }

    fn validate_enabled(&self, state: &State) -> anyhow::Result<()> {
        if matches!(self, ReceiveAddress::Arkade(_)) && state.arkade.is_none() {
            return Err(anyhow!("Arkade support is disabled"));
        }

        Ok(())
    }
}

impl Display for ReceiveAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReceiveAddress::Bark(address) => write!(f, "{address}"),
            ReceiveAddress::Arkade(address) => write!(f, "{address}"),
        }
    }
}

fn parse_receive_address(address: &str) -> anyhow::Result<ReceiveAddress> {
    if address.is_empty() {
        return Err(anyhow!("Ark address parameter is required"));
    }

    if let Ok(address) = address.parse::<ark::Address>() {
        return Ok(ReceiveAddress::Bark(address));
    }

    if let Ok(address) = address.parse::<ark_core::ArkAddress>() {
        return Ok(ReceiveAddress::Arkade(address));
    }

    Err(anyhow!("Invalid Ark address"))
}

fn resolve_receive_address(state: &State, address: &str) -> anyhow::Result<ResolvedReceiveAddress> {
    if let Ok(address) = parse_receive_address(address) {
        address.validate_network(state.network)?;
        address.validate_enabled(state)?;
        let identifier = address.to_string();
        return Ok(ResolvedReceiveAddress {
            address,
            identifier,
        });
    }

    let name = normalize_custom_address_name(address)?;
    let mut conn = state.db_pool.get()?;
    let custom_address = CustomAddress::get_by_name(&mut conn, &name)?
        .ok_or_else(|| anyhow!("Invalid Ark address"))?;
    let address = custom_address
        .ark_address
        .parse::<ark::Address>()
        .map(ReceiveAddress::Bark)
        .map_err(|_| anyhow!("Stored custom address target is invalid"))?;
    address.validate_network(state.network)?;

    Ok(ResolvedReceiveAddress {
        address,
        identifier: name,
    })
}

#[cfg(test)]
fn validate_ark_address(ark_address: &str) -> anyhow::Result<ark::Address> {
    match parse_receive_address(ark_address)? {
        ReceiveAddress::Bark(address) => Ok(address),
        ReceiveAddress::Arkade(_) => Err(anyhow!("Invalid Ark address")),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomAddressAuthMessageParams {
    pub name: String,
    pub ark_address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCustomAddressInvoiceRequest {
    pub name: String,
    pub ark_address: String,
    pub signature: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomAddressInvoiceResponse {
    pub id: i32,
    pub name: String,
    pub ark_address: String,
    pub fee_sats: u64,
    pub payment_hash: Option<String>,
    pub invoice: String,
    pub ark_payment_reference: Option<String>,
    pub state: String,
    pub active: bool,
}

pub async fn custom_address_auth_message(
    Query(params): Query<CustomAddressAuthMessageParams>,
    Extension(state): Extension<State>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let name = normalize_custom_address_name(&params.name).map_err(handle_anyhow_error)?;
    let ark_address =
        validate_custom_target_address(&params.ark_address, &state).map_err(handle_anyhow_error)?;
    let message = custom_address_signature_message(&state.domain, &name, &ark_address.to_string());

    Ok(Json(json!({
        "status": "OK",
        "name": name,
        "arkAddress": ark_address.to_string(),
        "message": message,
    })))
}

pub async fn create_custom_address_invoice(
    Extension(state): Extension<State>,
    Json(request): Json<CreateCustomAddressInvoiceRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let invoice = create_custom_address_invoice_impl(&state, request)
        .await
        .map_err(handle_anyhow_error)?;

    Ok(Json(json!({
        "status": "OK",
        "customAddress": format!("{}@{}", invoice.name, state.domain),
        "invoice": custom_address_invoice_response(invoice),
    })))
}

pub async fn get_custom_address_invoice(
    Path(id): Path<i32>,
    Extension(state): Extension<State>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut invoice = find_custom_address_invoice(&state, id)?;
    if invoice.state == InvoiceState::Pending as i32 {
        refresh_custom_address_invoice_receive_status(&state, &invoice).await?;
        invoice = find_custom_address_invoice(&state, id)?;
    }

    Ok(Json(json!({
        "status": "OK",
        "customAddress": format!("{}@{}", invoice.name, state.domain),
        "invoice": custom_address_invoice_response(invoice),
    })))
}

async fn create_custom_address_invoice_impl(
    state: &State,
    request: CreateCustomAddressInvoiceRequest,
) -> anyhow::Result<CustomAddressInvoice> {
    let name = normalize_custom_address_name(&request.name)?;
    let ark_address = validate_custom_target_address(&request.ark_address, state)?;

    if request.signature.len() > MAX_CUSTOM_SIGNATURE_LEN {
        warn!(
            "Rejected custom address invoice for name={} ark_address={} because signature_len={} exceeds max={}",
            name,
            ark_address,
            request.signature.len(),
            MAX_CUSTOM_SIGNATURE_LEN
        );
        return Err(anyhow!("Signature is too large"));
    }

    let auth_message =
        custom_address_signature_message(&state.domain, &name, &ark_address.to_string());
    info!(
        "Verifying custom address ownership for name={} ark_address={} domain={}",
        name, ark_address, state.domain
    );
    if !state
        .barkd
        .verify_address_message(
            ark_address.to_string(),
            auth_message.clone(),
            request.signature.clone(),
        )
        .await?
    {
        warn!(
            "Rejected custom address invoice for name={} ark_address={} because signature verification failed",
            name, ark_address
        );
        return Err(anyhow!("Invalid signature"));
    }

    {
        let mut conn = state.db_pool.get()?;
        let cancelled = CustomAddressInvoice::cancel_expired_pending_for_name(&mut conn, &name)?;
        if cancelled > 0 {
            info!(
                "Cancelled {} expired pending custom address invoice(s) for name={}",
                cancelled, name
            );
        }
        if CustomAddress::name_exists(&mut conn, &name)? {
            warn!(
                "Rejected custom address invoice for name={} because address is already active",
                name
            );
            return Err(anyhow!("Custom address is already taken"));
        }
        if CustomAddressInvoice::pending_name_exists(&mut conn, &name)? {
            warn!(
                "Rejected custom address invoice for name={} because a pending invoice already exists",
                name
            );
            return Err(anyhow!("Custom address already has a pending invoice"));
        }
    }

    let fee_receive_address = state.barkd.new_address().await?;
    let fee_sats = state.custom_address_fee_sats;
    info!(
        "Creating custom address fee invoice name={} ark_address={} fee_receive_address={} fee_sats={}",
        name, ark_address, fee_receive_address, fee_sats
    );
    let invoice = state
        .barkd
        .invoice_for_address(
            fee_sats,
            fee_receive_address.clone(),
            Some(format!("arkzap.me custom address {name}")),
        )
        .await?;

    if !invoice
        .amount_milli_satoshis()
        .is_some_and(|amount| amount == fee_sats * 1_000)
    {
        error!(
            "Custom address fee invoice amount mismatch name={} expected_msats={} actual_msats={:?}",
            name,
            fee_sats * 1_000,
            invoice.amount_milli_satoshis()
        );
        return Err(anyhow!("Invoice amount mismatch"));
    }

    let new_invoice = NewCustomAddressInvoice {
        name,
        ark_address: ark_address.to_string(),
        auth_message,
        signature: request.signature,
        fee_receive_address,
        bolt11: invoice.to_string(),
        amount_msats: (fee_sats * 1_000) as i64,
        payment_hash: Some(invoice.payment_hash().to_string()),
        preimage: String::new(),
        ark_payment_reference: None,
        state: InvoiceState::Pending as i32,
        expires_at: custom_address_invoice_expires_at(&invoice),
    };

    let mut conn = state.db_pool.get()?;
    let invoice = new_invoice.insert(&mut conn)?;
    info!(
        "Created custom address invoice id={} name={} ark_address={} fee_receive_address={} payment_hash={} amount_msats={} expires_at={:?}",
        invoice.id,
        invoice.name,
        invoice.ark_address,
        invoice.fee_receive_address,
        custom_address_invoice_payment_hash(&invoice),
        invoice.amount_msats,
        invoice.expires_at
    );
    Ok(invoice)
}

fn custom_address_invoice_expires_at(invoice: &Bolt11Invoice) -> Option<NaiveDateTime> {
    let reservation_expires_at = Utc::now().naive_utc() + CUSTOM_ADDRESS_INVOICE_EXPIRY;
    Some(
        invoice_expires_at(invoice)
            .map(|invoice_expires_at| invoice_expires_at.min(reservation_expires_at))
            .unwrap_or(reservation_expires_at),
    )
}

async fn refresh_custom_address_invoice_receive_status(
    state: &State,
    invoice: &CustomAddressInvoice,
) -> Result<(), (StatusCode, Json<Value>)> {
    let payment_hash = custom_address_invoice_payment_hash(invoice);
    info!(
        "Refreshing custom address invoice id={} name={} payment_hash={} amount_msats={} fee_receive_address={}",
        invoice.id,
        invoice.name,
        payment_hash,
        invoice.amount_msats,
        invoice.fee_receive_address
    );
    let receive = state
        .barkd
        .receive_status(&payment_hash)
        .await
        .map_err(|e| {
            error!(
                "Error refreshing custom address invoice {} payment_hash={payment_hash}: {e:#}",
                invoice.id
            );
            server_error_response()
        })?;

    let mut conn = state.db_pool.get().map_err(|e| {
        error!("DB connection error: {e}");
        server_error_response()
    })?;

    if let Some(receive) = receive.as_ref() {
        info!(
            "Custom address invoice id={} Lightning receive status payment_hash={} preimage_revealed={} finished={}",
            invoice.id,
            payment_hash,
            receive.preimage_revealed_at.is_some(),
            receive.finished_at.is_some()
        );
        if receive.preimage_revealed_at.is_some() {
            let activated = invoice
                .mark_lightning_settled_and_activate(
                    &mut conn,
                    receive.payment_preimage.to_string(),
                )
                .map_err(|e| {
                    error!(
                        "Error activating custom address invoice {} payment_hash={payment_hash}: {e:?}",
                        invoice.id
                    );
                    server_error_response()
                })?;
            if activated {
                info!(
                    "Activated custom address {} for {} from Lightning payment invoice_id={} payment_hash={}",
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
    } else {
        info!(
            "Custom address invoice id={} has no Lightning receive status payment_hash={}",
            invoice.id, payment_hash
        );
    }

    let ark_payment = state
        .barkd
        .received_ark_payment(
            &invoice.fee_receive_address,
            (invoice.amount_msats / 1_000) as u64,
        )
        .await
        .map_err(|e| {
            error!(
                "Error checking custom address invoice {} fee_receive_address={}: {e:#}",
                invoice.id, invoice.fee_receive_address
            );
            server_error_response()
        })?;
    info!(
        "Custom address invoice id={} Ark fee payment check fee_receive_address={} amount_sats={} paid={}",
        invoice.id,
        invoice.fee_receive_address,
        invoice.amount_msats / 1_000,
        ark_payment.is_some()
    );

    if let Some(ark_payment) = ark_payment {
        let activated = invoice
            .mark_ark_settled_and_activate(&mut conn, ark_payment.reference.clone())
            .map_err(|e| {
                error!(
                    "Error activating custom address invoice {} from Ark payment: {e:?}",
                    invoice.id
                );
                server_error_response()
            })?;
        if activated {
            info!(
                "Activated custom address {} for {} from Ark payment invoice_id={} fee_receive_address={} reference={} amount_sats={}",
                invoice.name,
                invoice.ark_address,
                invoice.id,
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
    } else if receive
        .as_ref()
        .is_some_and(|receive| receive.finished_at.is_some())
    {
        let cancelled = invoice.mark_cancelled(&mut conn).map_err(|e| {
            error!(
                "Error marking custom address invoice {} cancelled payment_hash={payment_hash}: {e:?}",
                invoice.id
            );
            server_error_response()
        })?;
        if cancelled {
            info!(
                "Cancelled terminal unpaid custom address invoice id={} payment_hash={}",
                invoice.id, payment_hash
            );
        }
    } else if invoice_has_expired(invoice) {
        let cancelled = invoice.mark_cancelled(&mut conn).map_err(|e| {
            error!(
                "Error marking expired custom address invoice {} cancelled payment_hash={payment_hash}: {e:?}",
                invoice.id
            );
            server_error_response()
        })?;
        if cancelled {
            info!(
                "Cancelled expired custom address invoice id={} payment_hash={} amount_msats={}",
                invoice.id, payment_hash, invoice.amount_msats
            );
        }
    } else {
        info!(
            "Custom address invoice id={} remains pending payment_hash={}",
            invoice.id, payment_hash
        );
    }

    Ok(())
}

fn find_custom_address_invoice(
    state: &State,
    id: i32,
) -> Result<CustomAddressInvoice, (StatusCode, Json<Value>)> {
    let mut conn = state.db_pool.get().map_err(|e| {
        error!("DB connection error: {e}");
        server_error_response()
    })?;

    CustomAddressInvoice::get_by_id(&mut conn, id)
        .map_err(|e| {
            error!("Error looking up custom address invoice {id}: {e:?}");
            server_error_response()
        })?
        .ok_or_else(|| (StatusCode::OK, Json(not_found_response())))
}

fn custom_address_invoice_response(invoice: CustomAddressInvoice) -> CustomAddressInvoiceResponse {
    CustomAddressInvoiceResponse {
        id: invoice.id,
        name: invoice.name,
        ark_address: invoice.fee_receive_address,
        fee_sats: (invoice.amount_msats / 1_000) as u64,
        payment_hash: invoice.payment_hash,
        invoice: invoice.bolt11,
        ark_payment_reference: invoice.ark_payment_reference,
        state: invoice_state_name(invoice.state).to_string(),
        active: invoice.state == InvoiceState::Settled as i32,
    }
}

fn custom_address_invoice_payment_hash(invoice: &CustomAddressInvoice) -> String {
    match invoice.payment_hash.as_deref() {
        Some(payment_hash) => payment_hash.to_string(),
        None => invoice.bolt11().payment_hash().to_string(),
    }
}

fn invoice_state_name(state: i32) -> &'static str {
    match state {
        state if state == InvoiceState::Pending as i32 => "pending",
        state if state == InvoiceState::Settled as i32 => "settled",
        state if state == InvoiceState::Cancelled as i32 => "cancelled",
        _ => "unknown",
    }
}

fn validate_custom_target_address(address: &str, state: &State) -> anyhow::Result<ark::Address> {
    let ark_address = address
        .parse::<ark::Address>()
        .map_err(|_| anyhow!("Custom addresses must target a Bark Ark address"))?;
    if ark_address.is_testnet() != (state.network != Network::Bitcoin) {
        return Err(anyhow!("Address is not valid for configured network"));
    }
    Ok(ark_address)
}

fn normalize_custom_address_name(name: &str) -> anyhow::Result<String> {
    let name = name.trim().to_ascii_lowercase();
    if name.len() < 3 || name.len() > 32 {
        return Err(anyhow!("Custom address name must be 3 to 32 characters"));
    }
    if !name.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
    }) {
        return Err(anyhow!(
            "Custom address name may only contain lowercase letters, numbers, hyphens, and underscores"
        ));
    }
    let first = name.as_bytes()[0];
    let last = name.as_bytes()[name.len() - 1];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit())
        || !(last.is_ascii_lowercase() || last.is_ascii_digit())
    {
        return Err(anyhow!(
            "Custom address name must start and end with a letter or number"
        ));
    }
    if parse_receive_address(&name).is_ok() {
        return Err(anyhow!("Custom address name cannot be an Ark address"));
    }

    Ok(name)
}

fn custom_address_signature_message(domain: &str, name: &str, ark_address: &str) -> String {
    format!("arkzap.me custom address\nname: {name}\ndomain: {domain}\nark_address: {ark_address}")
}

/// HTTP endpoint that provides the LNURL-pay metadata and parameters.
///
/// This is the entry point for the LNURL-pay protocol, served at the
/// .well-known/lnurlp/{ark_address} path.
///
/// # Parameters
/// * `ark_address` - Path parameter containing the Ark address portion of the Lightning address
/// * `state` - Application state with domain and configuration
///
/// # Returns
/// A LNURL PayResponse with callback URL and other parameters, or an error response
pub async fn get_lnurl_pay(
    Path(address): Path<String>,
    Extension(state): Extension<State>,
) -> Result<Json<PayResponse>, (StatusCode, Json<Value>)> {
    let resolved_address = match resolve_receive_address(&state, &address) {
        Ok(address) => address,
        Err(e) => {
            return Err(handle_anyhow_error(e));
        }
    };
    let min_sendable = resolved_address
        .address
        .min_sendable_msats(state.min_sendable);
    let address = resolved_address.identifier;

    let metadata = calc_metadata(&address, &state.domain);

    let callback = format!("https://{}/get-invoice/{address}", state.domain);

    let resp = PayResponse {
        callback,
        min_sendable,
        max_sendable: state.max_sendable,
        tag: Tag::PayRequest,
        metadata,
        comment_allowed: None,
        allows_nostr: Some(true),
        nostr_pubkey: Some(
            state
                .keys
                .public_key()
                .xonly()
                .expect("cant get xonly pubkey"),
        ),
    };

    Ok(Json(resp))
}

/// HTTP endpoint for verifying the status of a Lightning invoice payment.
///
/// This route is called by clients to check if an invoice has been paid.
///
/// # Parameters
/// * `address` and `pay_hash` - Path parameters for the receive address and payment hash
/// * `state` - Application state with LND client
///
/// # Returns
/// A JSON response indicating settlement status and preimage (if settled), or an error response
pub async fn verify(
    Path((address, pay_hash)): Path<(String, String)>,
    Extension(state): Extension<State>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let address = resolve_receive_address(&state, &address).map_err(handle_anyhow_error)?;
    let invoice_address = address.address.to_string();

    validate_hex_hash(&pay_hash, "Invalid payment hash")?;

    let mut invoice = find_invoice_by_payment_hash(&state, &pay_hash)?;

    if invoice.state() == InvoiceState::Pending as i32 {
        refresh_invoice_receive_status(&state, &invoice, &pay_hash).await?;
        invoice = find_invoice_by_payment_hash(&state, &pay_hash)?;
    }

    let bolt11 = invoice.bolt11();
    if invoice.address() != invoice_address {
        return Ok(Json(not_found_response()));
    }

    if invoice.state() == InvoiceState::Settled as i32 && !invoice.preimage().is_empty() {
        Ok(Json(json!({
            "status": "OK",
            "settled": true,
            "preimage": invoice.preimage(),
            "pr": bolt11,
        })))
    } else {
        Ok(Json(json!({
            "status": "OK",
            "settled": false,
            "preimage": null,
            "pr": bolt11,
        })))
    }
}

fn validate_hex_hash(hash: &str, reason: &str) -> Result<(), (StatusCode, Json<Value>)> {
    if hash.len() == 64 && hex::decode(hash).is_ok_and(|bytes| bytes.len() == 32) {
        Ok(())
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "status": "ERROR",
                "reason": reason,
            })),
        ))
    }
}

enum FoundInvoice {
    Bark(Invoice),
    Arkade(ArkadeInvoice),
}

impl FoundInvoice {
    fn address(&self) -> &str {
        match self {
            FoundInvoice::Bark(invoice) => &invoice.ark_address,
            FoundInvoice::Arkade(invoice) => &invoice.recipient_address,
        }
    }

    fn state(&self) -> i32 {
        match self {
            FoundInvoice::Bark(invoice) => invoice.state,
            FoundInvoice::Arkade(invoice) => invoice.state,
        }
    }

    fn preimage(&self) -> &str {
        match self {
            FoundInvoice::Bark(invoice) => &invoice.preimage,
            FoundInvoice::Arkade(invoice) => &invoice.preimage,
        }
    }

    fn bolt11(&self) -> Bolt11Invoice {
        match self {
            FoundInvoice::Bark(invoice) => invoice.bolt11(),
            FoundInvoice::Arkade(invoice) => invoice.bolt11(),
        }
    }
}

fn find_invoice_by_payment_hash(
    state: &State,
    payment_hash: &str,
) -> Result<FoundInvoice, (StatusCode, Json<Value>)> {
    let mut conn = state.db_pool.get().map_err(|e| {
        error!("DB connection error: {e}");
        server_error_response()
    })?;

    if let Some(invoice) = Invoice::get_by_payment_hash(&mut conn, payment_hash).map_err(|e| {
        error!("Error looking up invoice for payment_hash={payment_hash}: {e:?}");
        server_error_response()
    })? {
        return Ok(FoundInvoice::Bark(invoice));
    }

    if let Some(invoice) =
        ArkadeInvoice::get_by_payment_hash(&mut conn, payment_hash).map_err(|e| {
            error!("Error looking up Arkade invoice for payment_hash={payment_hash}: {e:?}");
            server_error_response()
        })?
    {
        return Ok(FoundInvoice::Arkade(invoice));
    }

    Err((StatusCode::OK, Json(not_found_response())))
}

async fn refresh_invoice_receive_status(
    state: &State,
    invoice: &FoundInvoice,
    payment_hash: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    match invoice {
        FoundInvoice::Bark(invoice) => {
            refresh_bark_invoice_receive_status(state, invoice, payment_hash).await
        }
        FoundInvoice::Arkade(invoice) => {
            refresh_arkade_invoice_receive_status(state, invoice, payment_hash).await
        }
    }
}

async fn refresh_bark_invoice_receive_status(
    state: &State,
    invoice: &Invoice,
    payment_hash: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    info!(
        "Refreshing Bark invoice payment status invoice_id={} payment_hash={} amount_msats={}",
        invoice.id, payment_hash, invoice.amount_msats
    );
    let receive = state
        .barkd
        .receive_status(payment_hash)
        .await
        .map_err(|e| {
            error!("Error refreshing receive status for payment_hash={payment_hash}: {e:#}");
            server_error_response()
        })?;

    let mut conn = state.db_pool.get().map_err(|e| {
        error!("DB connection error: {e}");
        server_error_response()
    })?;

    if let Some(receive) = receive {
        info!(
            "Bark invoice receive status invoice_id={} payment_hash={} preimage_revealed={} finished={}",
            invoice.id,
            payment_hash,
            receive.preimage_revealed_at.is_some(),
            receive.finished_at.is_some()
        );
        if receive.preimage_revealed_at.is_some() {
            let settled = invoice
                .mark_settled(&mut conn, receive.payment_preimage.to_string())
                .map_err(|e| {
                    error!("Error marking invoice settled for payment_hash={payment_hash}: {e:?}");
                    server_error_response()
                })?;
            if settled {
                info!(
                    "Marked Bark invoice settled invoice_id={} payment_hash={} amount_msats={}",
                    invoice.id, payment_hash, invoice.amount_msats
                );
            } else {
                warn!(
                    "Bark invoice settle was a no-op invoice_id={} payment_hash={}",
                    invoice.id, payment_hash
                );
            }
        } else if receive.finished_at.is_some() {
            let cancelled = invoice.mark_cancelled(&mut conn).map_err(|e| {
                error!("Error marking invoice cancelled for payment_hash={payment_hash}: {e:?}");
                server_error_response()
            })?;
            if cancelled {
                info!(
                    "Marked terminal unpaid Bark invoice cancelled invoice_id={} payment_hash={}",
                    invoice.id, payment_hash
                );
            } else {
                warn!(
                    "Bark invoice cancellation was a no-op invoice_id={} payment_hash={}",
                    invoice.id, payment_hash
                );
            }
        }
    } else if invoice_has_expired(invoice) {
        let cancelled = invoice.mark_cancelled(&mut conn).map_err(|e| {
            error!(
                "Error marking expired invoice cancelled for payment_hash={payment_hash}: {e:?}"
            );
            server_error_response()
        })?;
        if cancelled {
            info!(
                "Marked expired Bark invoice cancelled invoice_id={} payment_hash={} amount_msats={}",
                invoice.id, payment_hash, invoice.amount_msats
            );
        }
    } else {
        info!(
            "Bark invoice remains pending invoice_id={} payment_hash={}",
            invoice.id, payment_hash
        );
    }

    Ok(())
}

async fn refresh_arkade_invoice_receive_status(
    state: &State,
    invoice: &ArkadeInvoice,
    payment_hash: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    info!(
        "Refreshing Arkade invoice payment status invoice_id={} payment_hash={} amount_msats={} swap_id={}",
        invoice.id, payment_hash, invoice.amount_msats, invoice.swap_id
    );
    if invoice_has_expired(invoice) {
        let mut conn = state.db_pool.get().map_err(|e| {
            error!("DB connection error: {e}");
            server_error_response()
        })?;
        let cancelled = invoice.mark_cancelled(&mut conn).map_err(|e| {
            error!(
                "Error marking expired Arkade invoice cancelled for payment_hash={payment_hash}: {e:?}"
            );
            server_error_response()
        })?;
        if cancelled {
            info!(
                "Marked expired Arkade invoice cancelled invoice_id={} payment_hash={} amount_msats={} swap_id={}",
                invoice.id, payment_hash, invoice.amount_msats, invoice.swap_id
            );
        }
        return Ok(());
    }

    let Some(arkade) = state.arkade.as_ref() else {
        error!(
            "Cannot refresh Arkade invoice for payment_hash={payment_hash} swap_id={}: Arkade support is disabled",
            invoice.swap_id
        );
        return Ok(());
    };

    match arkade.claim_receive(&invoice.swap_id).await {
        Ok(preimage) => {
            let mut conn = state.db_pool.get().map_err(|e| {
                error!("DB connection error: {e}");
                server_error_response()
            })?;
            let settled = invoice
                .mark_settled(&mut conn, hex::encode(preimage))
                .map_err(|e| {
                    error!(
                        "Error marking Arkade invoice settled for payment_hash={payment_hash}: {e:?}"
                    );
                    server_error_response()
                })?;
            if settled {
                info!(
                    "Marked Arkade invoice settled invoice_id={} payment_hash={} amount_msats={} swap_id={}",
                    invoice.id, payment_hash, invoice.amount_msats, invoice.swap_id
                );
            } else {
                warn!(
                    "Arkade invoice settle was a no-op invoice_id={} payment_hash={} swap_id={}",
                    invoice.id, payment_hash, invoice.swap_id
                );
            }
        }
        Err(e) => {
            error!(
                "Error refreshing Arkade receive status for payment_hash={payment_hash} swap_id={}: {e:#}",
                invoice.swap_id
            );
        }
    }

    Ok(())
}

trait InvoiceExpiry {
    fn expires_at(&self) -> Option<NaiveDateTime>;
    fn bolt11(&self) -> Bolt11Invoice;
}

impl InvoiceExpiry for Invoice {
    fn expires_at(&self) -> Option<NaiveDateTime> {
        self.expires_at
    }

    fn bolt11(&self) -> Bolt11Invoice {
        self.bolt11()
    }
}

impl InvoiceExpiry for ArkadeInvoice {
    fn expires_at(&self) -> Option<NaiveDateTime> {
        self.expires_at
    }

    fn bolt11(&self) -> Bolt11Invoice {
        self.bolt11()
    }
}

impl InvoiceExpiry for CustomAddressInvoice {
    fn expires_at(&self) -> Option<NaiveDateTime> {
        self.expires_at
    }

    fn bolt11(&self) -> Bolt11Invoice {
        self.bolt11()
    }
}

fn invoice_has_expired(invoice: &impl InvoiceExpiry) -> bool {
    invoice
        .expires_at()
        .is_some_and(|expires_at| expires_at <= chrono::Utc::now().naive_utc())
        || invoice.bolt11().is_expired()
}

fn not_found_response() -> Value {
    json!({
        "status": "ERROR",
        "reason": "Not found",
    })
}

fn server_error_response() -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "status": "ERROR",
            "reason": "Server error",
        })),
    )
}

/// Utility function for converting anyhow errors to HTTP response format.
///
/// # Parameters
/// * `err` - The anyhow Error to convert
///
/// # Returns
/// A tuple containing a 400 Bad Request status code and a JSON error response
pub(crate) fn handle_anyhow_error(err: anyhow::Error) -> (StatusCode, Json<Value>) {
    let status = if err
        .chain()
        .any(|cause| cause.to_string().starts_with("barkd returned "))
    {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::BAD_REQUEST
    };
    let err = json!({
        "status": "ERROR",
        "reason": format!("{err}"),
    });
    (status, Json(err))
}

/// Fallback route handler that returns a 404 Not Found response
/// when a request is made to a non-existent route.
///
/// # Parameters
/// * `uri` - The URI of the request
///
/// # Returns
/// A 404 status code and a message indicating the route was not found
pub async fn fallback(uri: Uri) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, format!("No route for {}", uri))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::Html;
    use serde_json::json;

    #[tokio::test]
    async fn root_returns_arkzap_info_page() {
        let Html(body) = root().await;

        assert!(body.contains("<title>arkzap.me</title>"));
        assert!(body.contains("LNURL-pay infrastructure"));
        assert!(body.contains("/.well-known/lnurlp/:address"));
    }

    #[test]
    fn metadata_matches_lnurl_identifier_format() {
        assert_eq!(
            calc_metadata("ark1example", "example.com"),
            "[[\"text/identifier\",\"ark1example@example.com\"],[\"text/plain\",\"Sats for ark1example\"]]"
        );
    }

    #[test]
    fn amount_validation_accepts_whole_sats_within_bounds() {
        validate_amount_msats(10_000, 1_000, 100_000).unwrap();
    }

    #[test]
    fn amount_validation_rejects_out_of_bounds_amounts() {
        assert_eq!(
            validate_amount_msats(999, 1_000, 100_000)
                .unwrap_err()
                .to_string(),
            "Amount out of bounds"
        );
        assert_eq!(
            validate_amount_msats(101_000, 1_000, 100_000)
                .unwrap_err()
                .to_string(),
            "Amount out of bounds"
        );
    }

    #[test]
    fn amount_validation_rejects_non_whole_sat_amounts() {
        assert_eq!(
            validate_amount_msats(1_001, 1_000, 100_000)
                .unwrap_err()
                .to_string(),
            "Bark invoices must be denominated in whole sats"
        );
    }

    #[test]
    fn amount_validation_rejects_amounts_below_arkade_minimum() {
        assert_eq!(
            validate_amount_msats(332_000, ARKADE_MIN_SENDABLE_MSATS, 1_000_000)
                .unwrap_err()
                .to_string(),
            "Amount out of bounds"
        );
        validate_amount_msats(
            ARKADE_MIN_SENDABLE_MSATS,
            ARKADE_MIN_SENDABLE_MSATS,
            1_000_000,
        )
        .unwrap();
    }

    #[test]
    fn ark_address_validation_rejects_empty_or_invalid_addresses() {
        assert_eq!(
            validate_ark_address("").unwrap_err().to_string(),
            "Ark address parameter is required"
        );
        assert_eq!(
            validate_ark_address("not-an-ark-address")
                .unwrap_err()
                .to_string(),
            "Invalid Ark address"
        );
    }

    #[test]
    fn receive_address_validation_accepts_arkade_addresses() {
        let address = "tark1qqellv77udfmr20tun8dvju5vgudpf9vxe8jwhthrkn26fz96pawqfdy8nk05rsmrf8h94j26905e7n6sng8y059z8ykn2j5xcuw4xt846qj6x";
        let parsed = parse_receive_address(address).unwrap();
        assert_eq!(parsed.to_string(), address);
    }

    #[test]
    fn receive_address_validation_accepts_bark_addresses() {
        let address = "ark1pu6h30w3zqqplk5cnn4u9rl7ezmqcdyqjqxdkhn7q5acku3ctq48r7qzmgmxt6z3zqyps6f5kemv7aest5ekedtpmcl34n32vuagr4ufwdlw8ywzeagq7e4qqdv976";
        let parsed = parse_receive_address(address).unwrap();
        assert!(matches!(parsed, ReceiveAddress::Bark(_)));
        assert_eq!(parsed.to_string(), address);
    }

    #[test]
    fn arkade_addresses_use_at_least_333_sat_minimum() {
        let address = "tark1qqellv77udfmr20tun8dvju5vgudpf9vxe8jwhthrkn26fz96pawqfdy8nk05rsmrf8h94j26905e7n6sng8y059z8ykn2j5xcuw4xt846qj6x";
        let parsed = parse_receive_address(address).unwrap();

        assert_eq!(parsed.min_sendable_msats(1_000), ARKADE_MIN_SENDABLE_MSATS);
        assert_eq!(parsed.min_sendable_msats(500_000), 500_000);
    }

    #[test]
    fn arkade_address_validation_rejects_wrong_network() {
        let address = "tark1qqellv77udfmr20tun8dvju5vgudpf9vxe8jwhthrkn26fz96pawqfdy8nk05rsmrf8h94j26905e7n6sng8y059z8ykn2j5xcuw4xt846qj6x";
        let parsed = parse_receive_address(address).unwrap();

        parsed.validate_network(Network::Signet).unwrap();
        parsed.validate_network(Network::Regtest).unwrap();
        assert_eq!(
            parsed
                .validate_network(Network::Bitcoin)
                .unwrap_err()
                .to_string(),
            "Address is not valid for configured network"
        );
    }

    #[test]
    fn callback_validation_rejects_oversized_inputs() {
        let params = LnurlCallbackParams {
            nostr: Some("a".repeat(MAX_NOSTR_PARAM_LEN + 1)),
            ..Default::default()
        };
        assert_eq!(
            validate_callback_params(&params).unwrap_err().to_string(),
            "Nostr parameter is too large"
        );
    }

    #[test]
    fn missing_amount_errors_are_not_logged() {
        let err = anyhow!("Missing amount parameter");

        assert!(!should_log_invoice_error(&err));
    }

    #[test]
    fn invoice_generation_errors_are_logged() {
        let err = anyhow!("failed to generate barkd invoice for Ark address");

        assert!(should_log_invoice_error(&err));
    }

    #[test]
    fn upstream_barkd_errors_return_bad_gateway() {
        let err = anyhow!("barkd returned 500 Internal Server Error: body={{}}")
            .context("failed to generate barkd invoice for Ark address");

        let (status, Json(body)) = handle_anyhow_error(err);

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(
            body["reason"],
            "failed to generate barkd invoice for Ark address"
        );
    }

    #[test]
    fn empty_callback_strings_deserialize_to_none() {
        let params: LnurlCallbackParams = serde_json::from_value(json!({
            "amount": 1_000,
            "nostr": ""
        }))
        .unwrap();

        assert_eq!(params.amount, Some(1_000));
        assert_eq!(params.nostr, None);
    }
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

impl HealthResponse {
    /// Fabricate a status: pass response without checking database connectivity
    pub fn new_ok() -> Self {
        Self {
            status: String::from("pass"),
            version: String::from("0"),
        }
    }
}

/// IETF draft RFC for HTTP API Health Checks:
/// https://datatracker.ietf.org/doc/html/draft-inadarei-api-health-check
pub async fn health_check() -> Result<Json<HealthResponse>, (StatusCode, String)> {
    Ok(Json(HealthResponse::new_ok()))
}

pub fn empty_string_as_none<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => FromStr::from_str(s).map_err(de::Error::custom).map(Some),
    }
}
