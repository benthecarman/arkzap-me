use anyhow::Context;
use ark_bdk_wallet::Wallet as BdkArkWallet;
use ark_client::wallet::Persistence;
use ark_client::{
    Bip32KeyProvider, Blockchain, ChainSwapData, Client, Error as ArkadeError, OfflineClient,
    ReverseSwapData, SpendStatus, SubmarineSwapData, SwapAmount, SwapStatus, SwapStorage, TxStatus,
};
use ark_core::{ArkAddress, BoardingOutput, ExplorerUtxo};
use async_trait::async_trait;
use bitcoin::bip32::Xpriv;
use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::SecretKey;
use bitcoin::{Address, Amount, Network, Transaction, Txid, XOnlyPublicKey};
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;
use lightning_invoice::Bolt11Invoice;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::models::schema::arkade_swap_storage;
use diesel::prelude::*;

type DbPool = Pool<ConnectionManager<PgConnection>>;
type SdkWallet = BdkArkWallet<InMemoryBoardingPersistence>;
type SdkClient = Client<NoopBlockchain, SdkWallet, PostgresSwapStorage, Bip32KeyProvider>;

#[derive(Clone)]
pub struct ArkadeClient {
    client: Arc<SdkClient>,
    invoice_expiry_secs: Option<u64>,
    claim_timeout: Duration,
}

pub struct ArkadeInvoiceResult {
    pub invoice: Bolt11Invoice,
    pub swap_id: String,
}

impl ArkadeClient {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        db_pool: DbPool,
        xpriv: String,
        ark_server_url: String,
        boltz_url: String,
        esplora_url: String,
        network: Network,
        invoice_expiry_secs: Option<u64>,
        request_timeout_seconds: u64,
    ) -> anyhow::Result<Self> {
        let xpriv = Xpriv::from_str(&xpriv).context("invalid Arkade xpriv")?;
        let secp = Secp256k1::new();
        let wallet = Arc::new(
            BdkArkWallet::new_from_xpriv(
                xpriv,
                secp,
                network,
                &esplora_url,
                InMemoryBoardingPersistence::default(),
            )
            .context("failed to initialize Arkade BDK wallet")?,
        );

        let offline = OfflineClient::<
            NoopBlockchain,
            SdkWallet,
            PostgresSwapStorage,
            Bip32KeyProvider,
        >::new_with_bip32(
            "arkpay-me".to_string(),
            xpriv,
            None,
            Arc::new(NoopBlockchain),
            wallet,
            ark_server_url,
            Arc::new(PostgresSwapStorage::new(db_pool)),
            boltz_url,
            None,
            Duration::from_secs(request_timeout_seconds),
            None,
            vec![],
        );

        let client = offline
            .connect()
            .await
            .map_err(anyhow::Error::msg)
            .context("failed to connect Arkade client")?;

        Ok(Self {
            client: Arc::new(client),
            invoice_expiry_secs,
            claim_timeout: Duration::from_secs(request_timeout_seconds),
        })
    }

    pub async fn invoice_for_address(
        &self,
        amount_sat: u64,
        recipient_address: ArkAddress,
        description: Option<String>,
    ) -> anyhow::Result<ArkadeInvoiceResult> {
        let result = self
            .client
            .get_ln_invoice_for_address(
                SwapAmount::invoice(Amount::from_sat(amount_sat)),
                recipient_address,
                self.invoice_expiry_secs,
                description,
            )
            .await
            .map_err(anyhow::Error::msg)
            .context("failed to generate Arkade invoice for address")?;

        Ok(ArkadeInvoiceResult {
            invoice: result.invoice,
            swap_id: result.swap_id,
        })
    }

    pub async fn claim_receive(&self, swap_id: &str) -> anyhow::Result<[u8; 32]> {
        let result = tokio::time::timeout(self.claim_timeout, self.client.wait_for_vhtlc(swap_id))
            .await
            .context("Arkade receive is not ready to claim yet")?
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("failed to claim Arkade receive swap {swap_id}"))?;

        Ok(result.preimage)
    }
}

#[derive(Clone, Default)]
struct InMemoryBoardingPersistence {
    outputs: Arc<Mutex<Vec<(SecretKey, BoardingOutput)>>>,
}

impl Persistence for InMemoryBoardingPersistence {
    fn save_boarding_output(
        &self,
        sk: SecretKey,
        boarding_output: BoardingOutput,
    ) -> Result<(), ArkadeError> {
        self.outputs
            .lock()
            .map_err(|e| ArkadeError::wallet(format!("boarding persistence lock failed: {e}")))?
            .push((sk, boarding_output));
        Ok(())
    }

    fn load_boarding_outputs(&self) -> Result<Vec<BoardingOutput>, ArkadeError> {
        Ok(self
            .outputs
            .lock()
            .map_err(|e| ArkadeError::wallet(format!("boarding persistence lock failed: {e}")))?
            .iter()
            .map(|(_, output)| output.clone())
            .collect())
    }

    fn sk_for_pk(&self, pk: &XOnlyPublicKey) -> Result<SecretKey, ArkadeError> {
        let outputs = self
            .outputs
            .lock()
            .map_err(|e| ArkadeError::wallet(format!("boarding persistence lock failed: {e}")))?;
        outputs
            .iter()
            .find_map(|(sk, output)| (output.owner_pk() == *pk).then_some(*sk))
            .ok_or_else(|| ArkadeError::wallet(format!("boarding secret key not found for {pk}")))
    }
}

#[derive(Clone)]
struct NoopBlockchain;

impl Blockchain for NoopBlockchain {
    async fn find_outpoints(&self, _address: &Address) -> Result<Vec<ExplorerUtxo>, ArkadeError> {
        Err(ArkadeError::wallet(
            "NoopBlockchain does not support find_outpoints",
        ))
    }

    async fn find_tx(&self, _txid: &Txid) -> Result<Option<Transaction>, ArkadeError> {
        Err(ArkadeError::wallet(
            "NoopBlockchain does not support find_tx",
        ))
    }

    async fn get_tx_status(&self, _txid: &Txid) -> Result<TxStatus, ArkadeError> {
        Err(ArkadeError::wallet(
            "NoopBlockchain does not support get_tx_status",
        ))
    }

    async fn get_output_status(
        &self,
        _txid: &Txid,
        _vout: u32,
    ) -> Result<SpendStatus, ArkadeError> {
        Err(ArkadeError::wallet(
            "NoopBlockchain does not support get_output_status",
        ))
    }

    async fn broadcast(&self, _tx: &Transaction) -> Result<(), ArkadeError> {
        Err(ArkadeError::wallet(
            "NoopBlockchain does not support broadcast",
        ))
    }

    async fn get_fee_rate(&self) -> Result<f64, ArkadeError> {
        Err(ArkadeError::wallet(
            "NoopBlockchain does not support get_fee_rate",
        ))
    }

    async fn broadcast_package(&self, _txs: &[&Transaction]) -> Result<(), ArkadeError> {
        Err(ArkadeError::wallet(
            "NoopBlockchain does not support broadcast_package",
        ))
    }
}

#[derive(Clone)]
pub struct PostgresSwapStorage {
    db_pool: DbPool,
}

impl PostgresSwapStorage {
    pub fn new(db_pool: DbPool) -> Self {
        Self { db_pool }
    }

    fn insert<T: Serialize>(
        &self,
        swap_type: &str,
        id: String,
        data: T,
    ) -> Result<(), ArkadeError> {
        let mut conn = self.conn()?;
        let data = serde_json::to_value(data)
            .map_err(|e| ArkadeError::wallet(format!("failed to serialize swap: {e}")))?;

        diesel::insert_into(arkade_swap_storage::table)
            .values((
                arkade_swap_storage::swap_id.eq(id),
                arkade_swap_storage::swap_type.eq(swap_type),
                arkade_swap_storage::data.eq(data),
            ))
            .execute(&mut conn)
            .map(|_| ())
            .map_err(|e| ArkadeError::wallet(format!("failed to insert swap: {e}")))
    }

    fn get<T: DeserializeOwned>(
        &self,
        swap_type: &str,
        id: &str,
    ) -> Result<Option<T>, ArkadeError> {
        let mut conn = self.conn()?;
        let data = arkade_swap_storage::table
            .filter(arkade_swap_storage::swap_type.eq(swap_type))
            .filter(arkade_swap_storage::swap_id.eq(id))
            .select(arkade_swap_storage::data)
            .first::<serde_json::Value>(&mut conn)
            .optional()
            .map_err(|e| ArkadeError::wallet(format!("failed to get swap: {e}")))?;

        data.map(serde_json::from_value)
            .transpose()
            .map_err(|e| ArkadeError::wallet(format!("failed to deserialize swap: {e}")))
    }

    fn update<T: Serialize>(&self, swap_type: &str, id: &str, data: T) -> Result<(), ArkadeError> {
        let mut conn = self.conn()?;
        let data = serde_json::to_value(data)
            .map_err(|e| ArkadeError::wallet(format!("failed to serialize swap: {e}")))?;

        diesel::update(arkade_swap_storage::table)
            .filter(arkade_swap_storage::swap_type.eq(swap_type))
            .filter(arkade_swap_storage::swap_id.eq(id))
            .set((
                arkade_swap_storage::data.eq(data),
                arkade_swap_storage::updated_at.eq(diesel::dsl::now),
            ))
            .execute(&mut conn)
            .map(|_| ())
            .map_err(|e| ArkadeError::wallet(format!("failed to update swap: {e}")))
    }

    fn list<T: DeserializeOwned>(&self, swap_type: &str) -> Result<Vec<T>, ArkadeError> {
        let mut conn = self.conn()?;
        let rows = arkade_swap_storage::table
            .filter(arkade_swap_storage::swap_type.eq(swap_type))
            .select(arkade_swap_storage::data)
            .load::<serde_json::Value>(&mut conn)
            .map_err(|e| ArkadeError::wallet(format!("failed to list swaps: {e}")))?;

        rows.into_iter()
            .map(serde_json::from_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ArkadeError::wallet(format!("failed to deserialize swaps: {e}")))
    }

    fn remove<T: DeserializeOwned>(
        &self,
        swap_type: &str,
        id: &str,
    ) -> Result<Option<T>, ArkadeError> {
        let existing = self.get(swap_type, id)?;
        let mut conn = self.conn()?;
        diesel::delete(
            arkade_swap_storage::table
                .filter(arkade_swap_storage::swap_type.eq(swap_type))
                .filter(arkade_swap_storage::swap_id.eq(id)),
        )
        .execute(&mut conn)
        .map_err(|e| ArkadeError::wallet(format!("failed to remove swap: {e}")))?;
        Ok(existing)
    }

    fn conn(
        &self,
    ) -> Result<diesel::r2d2::PooledConnection<ConnectionManager<PgConnection>>, ArkadeError> {
        self.db_pool
            .get()
            .map_err(|e| ArkadeError::wallet(format!("failed to get DB connection: {e}")))
    }
}

#[async_trait]
impl SwapStorage for PostgresSwapStorage {
    async fn insert_submarine(
        &self,
        id: String,
        data: SubmarineSwapData,
    ) -> Result<(), ArkadeError> {
        self.insert("submarine", id, data)
    }

    async fn insert_reverse(&self, id: String, data: ReverseSwapData) -> Result<(), ArkadeError> {
        self.insert("reverse", id, data)
    }

    async fn get_submarine(&self, id: &str) -> Result<Option<SubmarineSwapData>, ArkadeError> {
        self.get("submarine", id)
    }

    async fn get_reverse(&self, id: &str) -> Result<Option<ReverseSwapData>, ArkadeError> {
        self.get("reverse", id)
    }

    async fn update_status_submarine(
        &self,
        id: &str,
        status: SwapStatus,
    ) -> Result<(), ArkadeError> {
        let mut data = self
            .get_submarine(id)
            .await?
            .ok_or_else(|| ArkadeError::wallet(format!("submarine swap not found: {id}")))?;
        data.status = status;
        self.update_submarine(id, data).await
    }

    async fn update_status_reverse(&self, id: &str, status: SwapStatus) -> Result<(), ArkadeError> {
        let mut data = self
            .get_reverse(id)
            .await?
            .ok_or_else(|| ArkadeError::wallet(format!("reverse swap not found: {id}")))?;
        data.status = status;
        self.update_reverse(id, data).await
    }

    async fn update_submarine(&self, id: &str, data: SubmarineSwapData) -> Result<(), ArkadeError> {
        self.update("submarine", id, data)
    }

    async fn update_reverse(&self, id: &str, data: ReverseSwapData) -> Result<(), ArkadeError> {
        self.update("reverse", id, data)
    }

    async fn list_all_submarine(&self) -> Result<Vec<SubmarineSwapData>, ArkadeError> {
        self.list("submarine")
    }

    async fn list_all_reverse(&self) -> Result<Vec<ReverseSwapData>, ArkadeError> {
        self.list("reverse")
    }

    async fn remove_submarine(&self, id: &str) -> Result<Option<SubmarineSwapData>, ArkadeError> {
        self.remove("submarine", id)
    }

    async fn remove_reverse(&self, id: &str) -> Result<Option<ReverseSwapData>, ArkadeError> {
        self.remove("reverse", id)
    }

    async fn insert_chain(&self, id: String, data: ChainSwapData) -> Result<(), ArkadeError> {
        self.insert("chain", id, data)
    }

    async fn get_chain(&self, id: &str) -> Result<Option<ChainSwapData>, ArkadeError> {
        self.get("chain", id)
    }

    async fn update_status_chain(&self, id: &str, status: SwapStatus) -> Result<(), ArkadeError> {
        let mut data = self
            .get_chain(id)
            .await?
            .ok_or_else(|| ArkadeError::wallet(format!("chain swap not found: {id}")))?;
        data.status = status;
        self.update_chain(id, data).await
    }

    async fn update_chain(&self, id: &str, data: ChainSwapData) -> Result<(), ArkadeError> {
        self.update("chain", id, data)
    }

    async fn list_all_chain(&self) -> Result<Vec<ChainSwapData>, ArkadeError> {
        self.list("chain")
    }

    async fn remove_chain(&self, id: &str) -> Result<Option<ChainSwapData>, ArkadeError> {
        self.remove("chain", id)
    }
}
