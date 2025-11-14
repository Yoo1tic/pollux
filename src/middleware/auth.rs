use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::{HeaderMap, StatusCode, request::Parts};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::config::CONFIG;

/// Ensure the inbound request is authorized.
/// Accepts either:
/// - Query string: `?key=...`
/// - Header: `x-goog-api-key: ...`
///   Requires server key to be configured via `NEXUS_KEY`.
pub fn ensure_authorized(headers: &HeaderMap, query: Option<&str>) -> Result<(), Response> {
    let expected = CONFIG.nexus_key.as_str();

    // 1) header: x-goog-api-key
    if let Some(hv) = headers.get("x-goog-api-key").and_then(|v| v.to_str().ok())
        && hv == expected
    {
        return Ok(());
    }

    // 2) header: Authorization: Bearer <key>
    if let Some(auth) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        let auth = auth.trim();
        if let Some(token) = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
            && token == expected
        {
            return Ok(());
        }
    }

    // 3) query: key=...
    if let Some(qs) = query {
        for (k, v) in url::form_urlencoded::parse(qs.as_bytes()) {
            if k == "key" && v == expected {
                return Ok(());
            }
        }
    }

    Err((
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "unauthorized", "reason": "invalid or missing key"})),
    )
        .into_response())
}

#[derive(Debug, Clone, Copy)]
pub struct RequireKeyAuth;

impl<S> FromRequestParts<S> for RequireKeyAuth
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let headers = &parts.headers;
        let query = parts.uri.query();
        ensure_authorized(headers, query)?;
        Ok(Self)
    }
}
