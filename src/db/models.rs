use crate::google_oauth::credentials::GoogleCredential;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Persistent representation of a Google credential row.
/// Includes an auto-increment `id` primary key and a `status` flag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DbCredential {
    pub id: i64,
    pub email: Option<String>,
    pub client_id: String,
    pub client_secret: String,
    pub project_id: String,
    pub scopes: Option<Vec<String>>, // stored as JSON/text in SQLite
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expiry: DateTime<Utc>,
    pub status: bool,
}

impl From<GoogleCredential> for DbCredential {
    fn from(g: GoogleCredential) -> Self {
        Self {
            id: 0, // to be set by DB AUTOINCREMENT on insert
            email: g.email,
            client_id: g.client_id,
            client_secret: g.client_secret,
            project_id: g.project_id,
            scopes: g.scopes,
            refresh_token: g.refresh_token,
            access_token: g.access_token,
            expiry: g.expiry,
            status: true,
        }
    }
}

impl From<DbCredential> for GoogleCredential {
    fn from(d: DbCredential) -> Self {
        GoogleCredential {
            email: d.email,
            client_id: d.client_id,
            client_secret: d.client_secret,
            project_id: d.project_id,
            scopes: d.scopes,
            refresh_token: d.refresh_token,
            access_token: d.access_token,
            expiry: d.expiry,
        }
    }
}

