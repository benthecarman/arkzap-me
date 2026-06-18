use anyhow::{anyhow, Context};
use ark::bitcoin::secp256k1::schnorr;
use bark_rest_client::apis::configuration::Configuration;
use bark_rest_client::apis::{history_api, lightning_api, wallet_api};
use bark_rest_client::models::{
    LightningInvoiceForAddressRequest, LightningReceiveInfo, PaymentMethod,
};
use lightning_invoice::Bolt11Invoice;
use log::info;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

#[derive(Clone)]
pub struct BarkdClient {
    config: Configuration,
}

impl BarkdClient {
    pub fn new(base_url: String, bearer_token: Option<String>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .context("failed to build barkd HTTP client")?;

        let config = Configuration {
            base_path: base_url.trim_end_matches('/').to_string(),
            client,
            bearer_access_token: bearer_token,
            ..Configuration::default()
        };

        Ok(Self { config })
    }

    pub async fn invoice_for_address(
        &self,
        amount_sat: u64,
        address: String,
        description: Option<String>,
    ) -> anyhow::Result<Bolt11Invoice> {
        info!(
            "Requesting barkd Lightning invoice amount_sats={} address={} description_present={}",
            amount_sat,
            address,
            description.is_some()
        );
        let info = lightning_api::generate_invoice_for_address(
            &self.config,
            LightningInvoiceForAddressRequest {
                amount_sat,
                address: address.clone(),
                description,
            },
        )
        .await
        .map_err(barkd_error)
        .context("failed to generate barkd invoice for Ark address")?;

        let invoice = Bolt11Invoice::from_str(&info.invoice)
            .context("barkd returned invalid BOLT11 invoice")?;
        info!(
            "Generated barkd Lightning invoice amount_sats={} address={} payment_hash={} expires_at={:?}",
            amount_sat,
            address,
            invoice.payment_hash(),
            invoice.expires_at()
        );
        Ok(invoice)
    }

    pub async fn new_address(&self) -> anyhow::Result<String> {
        info!("Requesting new barkd receive address");
        let response = wallet_api::address(&self.config)
            .await
            .map_err(barkd_error)
            .context("failed to generate barkd receive address")?;

        info!(
            "Generated new barkd receive address address={}",
            response.address
        );
        Ok(response.address)
    }

    pub async fn verify_address_message(
        &self,
        address: String,
        message: String,
        signature: String,
    ) -> anyhow::Result<bool> {
        let address = address
            .parse::<ark::Address>()
            .context("invalid Ark address")?;
        let signature =
            schnorr::Signature::from_str(&signature).context("invalid Schnorr signature")?;

        Ok(address
            .verify_message(message.as_bytes(), &signature)
            .is_ok())
    }

    pub async fn receive_status(
        &self,
        identifier: &str,
    ) -> anyhow::Result<Option<LightningReceiveInfo>> {
        info!("Requesting barkd receive status identifier={identifier}");
        match lightning_api::get_receive_status(&self.config, identifier).await {
            Ok(status) => {
                info!(
                    "Received barkd receive status identifier={} preimage_revealed={} finished={}",
                    identifier,
                    status.preimage_revealed_at.is_some(),
                    status.finished_at.is_some()
                );
                Ok(Some(status))
            }
            Err(bark_rest_client::apis::Error::ResponseError(resp))
                if resp.status == reqwest::StatusCode::NOT_FOUND =>
            {
                info!("No barkd receive status found identifier={identifier}");
                Ok(None)
            }
            Err(e) => Err(barkd_error(e).context("failed to get barkd receive status")),
        }
    }

    pub async fn pending_receives(&self) -> anyhow::Result<Vec<LightningReceiveInfo>> {
        info!("Requesting barkd pending receive statuses");
        let receives = lightning_api::list_receive_statuses(&self.config)
            .await
            .map_err(barkd_error)
            .context("failed to list barkd receive statuses")?;
        info!("Listed {} barkd pending receive status(es)", receives.len());
        Ok(receives)
    }

    pub async fn has_received_ark_payment(
        &self,
        address: &str,
        min_amount_sat: u64,
    ) -> anyhow::Result<bool> {
        info!(
            "Checking barkd wallet history for Ark payment address={} min_amount_sats={}",
            address, min_amount_sat
        );
        let movements = history_api::list(&self.config)
            .await
            .map_err(barkd_error)
            .context("failed to list barkd wallet history")?;

        let movement_count = movements.len();
        let received = movements.into_iter().any(|movement| {
            movement.received_on.into_iter().any(|destination| {
                matches!(
                    destination.destination,
                    PaymentMethod::Ark(received_address)
                        if received_address == address && destination.amount.to_sat() >= min_amount_sat
                )
            })
        });
        info!(
            "Checked barkd wallet history for Ark payment address={} min_amount_sats={} movements={} received={}",
            address, min_amount_sat, movement_count, received
        );
        Ok(received)
    }
}

fn barkd_error<T: fmt::Debug>(err: bark_rest_client::apis::Error<T>) -> anyhow::Error {
    match err {
        bark_rest_client::apis::Error::ResponseError(resp) => anyhow!(
            "barkd returned {}: body={}, parsed_error={:?}",
            resp.status,
            resp.content,
            resp.entity
        ),
        err => anyhow!("{err}"),
    }
}
