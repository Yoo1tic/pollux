use crate::config::CONFIG;
use axum::{
    Json,
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::headers::{Authorization, HeaderMapExt, authorization::Bearer};
use serde_json::json;
use subtle::ConstantTimeEq;

fn extract_header_token(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(k) = headers.get("x-goog-api-key").and_then(|v| v.to_str().ok()) {
        return Some(k.to_string());
    }
    headers
        .typed_get::<Authorization<Bearer>>()
        .map(|auth| auth.token().to_string())
}

fn extract_query_token(query: Option<&str>) -> Option<String> {
    query.and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(k, _)| k == "key")
            .map(|(_, v)| v.into_owned())
    })
}

#[derive(Debug, Clone, Copy)]
pub struct RequireKeyAuth;

impl<S> FromRequestParts<S> for RequireKeyAuth
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let token =
            extract_header_token(&parts.headers).or_else(|| extract_query_token(parts.uri.query()));

        match token {
            Some(key) => {
                let expected = CONFIG.nexus_key.as_str();
                if key.as_bytes().ct_eq(expected.as_bytes()).into() {
                    Ok(RequireKeyAuth)
                } else {
                    Err(AuthError::InvalidKey)
                }
            }
            None => Err(AuthError::MissingKey),
        }
    }
}

pub enum AuthError {
    MissingKey,
    InvalidKey,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, reason) = match self {
            AuthError::MissingKey => (StatusCode::UNAUTHORIZED, "Missing API key"),
            AuthError::InvalidKey => (StatusCode::UNAUTHORIZED, "Invalid API key"),
        };
        (
            status,
            Json(json!({ "error": "unauthorized", "reason": reason })),
        )
            .into_response()
    }
}
