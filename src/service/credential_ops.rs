use crate::config::CONFIG;
use crate::db::sqlite::CredentialsStorage;
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use crate::google_oauth::service::{GoogleOauthService, RefreshJob};
use crate::service::credential_manager::CredentialId;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub struct CredentialOps {
    refresh_tx: mpsc::UnboundedSender<RefreshJob>,
    storage: CredentialsStorage,
}

impl CredentialOps {
    pub async fn new() -> Result<Self, NexusError> {
        let svc = GoogleOauthService::new();
        let refresh_tx = svc.refresh_tx();

        let connect_opts =
            SqliteConnectOptions::from_str(CONFIG.database_url.as_str())?.create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(connect_opts)
            .await?;
        let storage = CredentialsStorage::new(pool);
        storage.init_schema().await?;

        Ok(Self {
            refresh_tx,
            storage,
        })
    }

    pub async fn load_active(&self) -> Result<Vec<(CredentialId, GoogleCredential)>, NexusError> {
        let rows = self.storage.list_active().await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.id as CredentialId, row.into()))
            .collect())
    }

    pub fn enqueue_refresh(
        &self,
        cred: GoogleCredential,
    ) -> Result<oneshot::Receiver<Result<GoogleCredential, NexusError>>, NexusError> {
        let (tx_done, rx_done) = oneshot::channel();
        self.refresh_tx
            .send(RefreshJob {
                cred,
                respond_to: tx_done,
            })
            .map_err(|e| NexusError::RactorError(format!("send refresh job failed: {}", e)))?;
        Ok(rx_done)
    }

    pub async fn upsert(&self, cred: GoogleCredential, status: bool) -> Result<CredentialId, NexusError> {
        let id = self.storage.upsert(cred, status).await?;
        Ok(id as CredentialId)
    }

    pub async fn update_by_id(
        &self,
        id: CredentialId,
        cred: GoogleCredential,
        status: bool,
    ) -> Result<(), NexusError> {
        self.storage.update_by_id(id, cred, status).await
    }

    pub async fn set_status(&self, id: CredentialId, status: bool) -> Result<(), NexusError> {
        self.storage.set_status(id, status).await
    }
}
