use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr};

/// Basic (core) configuration managed by Figment.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BasicConfig {
    /// HTTP server listen address (e.g., "0.0.0.0", "127.0.0.1").
    /// TOML: `basic.listen_addr`. Default: `0.0.0.0`.
    #[serde(default = "default_listen_ip")]
    pub listen_addr: IpAddr,

    /// HTTP server listen port.
    /// TOML: `basic.listen_port`. Default: `8188`.
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,

    /// Database URL for SQLite.
    /// TOML: `basic.database_url`. Default: `sqlite://data.db`.
    #[serde(default)]
    pub database_url: String,

    /// Log level for tracing subscriber initialization (e.g., "error", "warn", "info", "debug", "trace").
    /// TOML: `basic.loglevel`. Default: `info`.
    #[serde(default)]
    pub loglevel: String,

    /// Authentication key for inbound request validation (required, non-empty).
    /// TOML: `basic.pollux_key`. Must be provided.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_string_lax")]
    pub pollux_key: String,

    /// Whether OAuth CSRF/PKCE cookies are marked insecure (`Secure=false`).
    /// TOML: `basic.insecure_cookie`. Default: `false`.
    ///
    /// Keep `false` in production/HTTPS. Set `true` only for local plain-HTTP testing.
    #[serde(default)]
    pub insecure_cookie: bool,
}

impl Default for BasicConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_ip(),
            listen_port: default_listen_port(),
            database_url: "sqlite://data.db".to_string(),
            loglevel: "info".to_string(),
            // No insecure default. `Config::from_toml()` enforces non-empty.
            pollux_key: "".to_string(),
            insecure_cookie: false,
        }
    }
}

fn deserialize_string_lax<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let v = Value::deserialize(deserializer)?;

    match v {
        Value::String(s) => Ok(s),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(serde::de::Error::custom(
            "expected a string or a number for basic.pollux_key",
        )),
    }
}

/// Default IP address for the HTTP server listen address.
fn default_listen_ip() -> IpAddr {
    Ipv4Addr::new(0, 0, 0, 0).into()
}

/// Default port for the HTTP server.
fn default_listen_port() -> u16 {
    8188
}
