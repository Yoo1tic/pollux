//! Database module: models and schema for persistent storage.
//!
//! Layout:
//! - `models.rs`: Rust structs mirroring DB rows and conversions
//! - `schema.rs`: SQL DDL for initializing the database (SQLite-first)

pub mod models;
pub mod schema;
pub mod sqlite;

pub use models::DbCredential;
pub use schema::SQLITE_INIT;
pub use sqlite::{CredentialsStorage, SqlitePool};
