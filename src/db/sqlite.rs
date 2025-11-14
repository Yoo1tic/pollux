use crate::db::models::DbCredential;
use crate::db::schema::SQLITE_INIT;
use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use chrono::{DateTime, Utc};
use sqlx::sqlite::SqliteRow;
use sqlx::{Pool, Row, Sqlite};

pub type SqlitePool = Pool<Sqlite>;

#[derive(Clone)]
pub struct CredentialsStorage {
    pool: SqlitePool,
}

impl CredentialsStorage {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Initialize the schema by executing the bundled DDL.
    pub async fn init_schema(&self) -> Result<(), NexusError> {
        // execute multiple statements safely (SQLite supports multi-commands but sqlx::query doesn't)
        for stmt in SQLITE_INIT.split(';') {
            let s = stmt.trim();
            if s.is_empty() {
                continue;
            }
            sqlx::query(s).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Upsert by unique project_id. Returns the row id.
    /// Uses SQLite `INSERT ... ON CONFLICT(project_id) DO UPDATE`.
    pub async fn upsert(&self, cred: GoogleCredential, status: bool) -> Result<i64, NexusError> {
        let scopes_json = cred
            .scopes
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let expiry = cred.expiry.to_rfc3339();
        let status_i = if status { 1 } else { 0 };
        // Perform upsert
        sqlx::query(
            r#"
            INSERT INTO credentials (
                email, client_id, client_secret, project_id, scopes,
                refresh_token, access_token, expiry, status
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(project_id) DO UPDATE SET
                email=excluded.email,
                client_id=excluded.client_id,
                client_secret=excluded.client_secret,
                scopes=excluded.scopes,
                refresh_token=excluded.refresh_token,
                access_token=excluded.access_token,
                expiry=excluded.expiry,
                status=excluded.status
            "#,
        )
        .bind(cred.email)
        .bind(cred.client_id)
        .bind(cred.client_secret)
        .bind(cred.project_id.clone())
        .bind(scopes_json)
        .bind(cred.refresh_token)
        .bind(cred.access_token)
        .bind(expiry)
        .bind(status_i)
        .execute(&self.pool)
        .await?;

        // Fetch id after upsert
        let rec: (i64,) = sqlx::query_as("SELECT id FROM credentials WHERE project_id = ?")
            .bind(cred.project_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(rec.0)
    }

    /// Batch upsert using a single transaction. Returns ids in the same order.
    pub async fn upsert_many(
        &self,
        items: Vec<(GoogleCredential, bool)>,
    ) -> Result<Vec<i64>, NexusError> {
        let mut tx = self.pool.begin().await?;
        let mut ids = Vec::with_capacity(items.len());

        for (cred, status) in items.into_iter() {
            let scopes_json = cred
                .scopes
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
            let expiry = cred.expiry.to_rfc3339();
            let status_i = if status { 1 } else { 0 };

            sqlx::query(
                r#"
                INSERT INTO credentials (
                    email, client_id, client_secret, project_id, scopes,
                    refresh_token, access_token, expiry, status
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(project_id) DO UPDATE SET
                    email=excluded.email,
                    client_id=excluded.client_id,
                    client_secret=excluded.client_secret,
                    scopes=excluded.scopes,
                    refresh_token=excluded.refresh_token,
                    access_token=excluded.access_token,
                    expiry=excluded.expiry,
                    status=excluded.status
                "#,
            )
            .bind(cred.email)
            .bind(cred.client_id)
            .bind(cred.client_secret)
            .bind(cred.project_id.clone())
            .bind(scopes_json)
            .bind(cred.refresh_token)
            .bind(cred.access_token)
            .bind(expiry)
            .bind(status_i)
            .execute(&mut *tx)
            .await?;

            let rec: (i64,) = sqlx::query_as("SELECT id FROM credentials WHERE project_id = ?")
                .bind(cred.project_id)
                .fetch_one(&mut *tx)
                .await?;
            ids.push(rec.0);
        }

        tx.commit().await?;
        Ok(ids)
    }

    pub async fn get_by_id(&self, id: i64) -> Result<DbCredential, NexusError> {
        let row = sqlx::query(
            r#"SELECT id, email, client_id, client_secret, project_id, scopes,
               refresh_token, access_token, expiry, status
               FROM credentials WHERE id = ?"#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Self::row_to_model(row)
    }

    pub async fn get_by_project_id(&self, project_id: &str) -> Result<DbCredential, NexusError> {
        let row = sqlx::query(
            r#"SELECT id, email, client_id, client_secret, project_id, scopes,
               refresh_token, access_token, expiry, status
               FROM credentials WHERE project_id = ?"#,
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        Self::row_to_model(row)
    }

    pub async fn list_active(&self) -> Result<Vec<DbCredential>, NexusError> {
        let rows = sqlx::query(
            r#"SELECT id, email, client_id, client_secret, project_id, scopes,
               refresh_token, access_token, expiry, status
               FROM credentials WHERE status = 1 ORDER BY id"#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Self::row_to_model).collect()
    }

    pub async fn set_status(&self, id: i64, status: bool) -> Result<(), NexusError> {
        let status_i = if status { 1 } else { 0 };
        sqlx::query("UPDATE credentials SET status = ? WHERE id = ?")
            .bind(status_i)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Update all credential fields by id (except id itself).
    pub async fn update_by_id(
        &self,
        id: i64,
        cred: GoogleCredential,
        status: bool,
    ) -> Result<(), NexusError> {
        let scopes_json = cred
            .scopes
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let expiry = cred.expiry.to_rfc3339();
        let status_i = if status { 1 } else { 0 };
        sqlx::query(
            r#"UPDATE credentials SET
                email = ?,
                client_id = ?,
                client_secret = ?,
                project_id = ?,
                scopes = ?,
                refresh_token = ?,
                access_token = ?,
                expiry = ?,
                status = ?
              WHERE id = ?"#,
        )
        .bind(cred.email)
        .bind(cred.client_id)
        .bind(cred.client_secret)
        .bind(cred.project_id)
        .bind(scopes_json)
        .bind(cred.refresh_token)
        .bind(cred.access_token)
        .bind(expiry)
        .bind(status_i)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn row_to_model(row: SqliteRow) -> Result<DbCredential, NexusError> {
        let id: i64 = row.try_get("id")?;
        let email: Option<String> = row.try_get("email")?;
        let client_id: String = row.try_get("client_id")?;
        let client_secret: String = row.try_get("client_secret")?;
        let project_id: String = row.try_get("project_id")?;
        let scopes_json: Option<String> = row.try_get("scopes")?;
        let refresh_token: String = row.try_get("refresh_token")?;
        let access_token: Option<String> = row.try_get("access_token")?;
        let expiry_str: String = row.try_get("expiry")?;
        let status_i: i64 = row.try_get("status")?;

        let scopes: Option<Vec<String>> = match scopes_json {
            Some(s) => {
                Some(serde_json::from_str(&s).map_err(|e| sqlx::Error::Decode(Box::new(e)))?)
            }
            None => None,
        };
        let expiry: DateTime<Utc> = chrono::DateTime::parse_from_rfc3339(&expiry_str)
            .map_err(|e| sqlx::Error::Decode(Box::new(e)))?
            .with_timezone(&Utc);
        let status = status_i != 0;

        Ok(DbCredential {
            id,
            email,
            client_id,
            client_secret,
            project_id,
            scopes,
            refresh_token,
            access_token,
            expiry,
            status,
        })
    }
}
