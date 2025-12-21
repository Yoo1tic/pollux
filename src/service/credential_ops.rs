use crate::config::CONFIG;
use crate::db::sqlite::CredentialsStorage;
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use crate::service::credential_manager::CredentialId;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{str::FromStr, time::Duration};

#[derive(Clone)]
pub struct CredentialOps {
    storage: CredentialsStorage,
}

impl CredentialOps {
    pub async fn new() -> Result<Self, NexusError> {
        let connect_opts = SqliteConnectOptions::from_str(CONFIG.database_url.as_str())?
            .create_if_missing(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new().connect_with(connect_opts).await?;
        let storage = CredentialsStorage::new(pool);
        storage.init_schema().await?;

        Ok(Self { storage })
    }

    pub async fn load_active(&self) -> Result<Vec<(CredentialId, GoogleCredential)>, NexusError> {
        let rows = self.storage.list_active().await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.id as CredentialId, row.into()))
            .collect())
    }

    pub async fn upsert(
        &self,
        cred: GoogleCredential,
        status: bool,
    ) -> Result<CredentialId, NexusError> {
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
