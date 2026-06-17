use anyhow::{anyhow, Context};
use bark_rest_client::apis::configuration::Configuration;
use bark_rest_client::apis::{lightning_api, wallet_api};
use bark_rest_client::models::{
    AddressVerifyMessageRequest, LightningInvoiceForAddressRequest, LightningReceiveInfo,
};
use lightning_invoice::Bolt11Invoice;
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
        let info = lightning_api::generate_invoice_for_address(
            &self.config,
            LightningInvoiceForAddressRequest {
                amount_sat,
                address,
                description,
            },
        )
        .await
        .map_err(barkd_error)
        .context("failed to generate barkd invoice for Ark address")?;

        Bolt11Invoice::from_str(&info.invoice).context("barkd returned invalid BOLT11 invoice")
    }

    pub async fn new_address(&self) -> anyhow::Result<String> {
        let response = wallet_api::address(&self.config)
            .await
            .map_err(barkd_error)
            .context("failed to generate barkd receive address")?;

        Ok(response.address)
    }

    pub async fn verify_address_message(
        &self,
        address: String,
        message: String,
        signature: String,
    ) -> anyhow::Result<bool> {
        let response = wallet_api::verify_address_message(
            &self.config,
            AddressVerifyMessageRequest {
                address,
                message,
                signature,
            },
        )
        .await
        .map_err(barkd_error)
        .context("failed to verify Bark address message")?;

        Ok(response.valid)
    }

    pub async fn receive_status(
        &self,
        identifier: &str,
    ) -> anyhow::Result<Option<LightningReceiveInfo>> {
        match lightning_api::get_receive_status(&self.config, identifier).await {
            Ok(status) => Ok(Some(status)),
            Err(bark_rest_client::apis::Error::ResponseError(resp))
                if resp.status == reqwest::StatusCode::NOT_FOUND =>
            {
                Ok(None)
            }
            Err(e) => Err(barkd_error(e).context("failed to get barkd receive status")),
        }
    }

    pub async fn pending_receives(&self) -> anyhow::Result<Vec<LightningReceiveInfo>> {
        lightning_api::list_receive_statuses(&self.config)
            .await
            .map_err(barkd_error)
            .context("failed to list barkd receive statuses")
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
