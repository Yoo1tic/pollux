use axum::{
    Json,
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error as ThisError;

use super::IsRetryable;
use crate::providers::UPSTREAM_BODY_PREVIEW_CHARS;
use pollux_schema::{CodexErrorBody, OpenaiResponsesErrorBody, OpenaiResponsesErrorObject};

#[derive(Debug, ThisError)]
pub(crate) enum CodexError {
    #[error("Request rejected")]
    RequestRejected {
        status: StatusCode,
        body: OpenaiResponsesErrorObject,
        debug_message: Option<String>,
    },

    /// No usable credential is currently available.
    #[error("No available credential")]
    NoAvailableCredential,

    /// Upstream error that matched a provider mapping rule.
    #[error("Upstream mapped error: status={status}, body={body:?}")]
    UpstreamMappedError {
        status: StatusCode,
        body: CodexErrorBody,
    },

    /// Upstream fallback error (rule unmatched or body unstructured).
    #[error("Upstream fallback error: status={status}, body={body:.200}")]
    UpstreamFallbackError {
        status: StatusCode,
        /// Raw upstream body is preserved for internal diagnostics/logging only.
        body: String,
    },

    /// Transport-level failure (DNS, connect, timeouts, etc).
    #[error("HTTP request error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("Stream protocol error: {0}")]
    StreamProtocolError(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<JsonRejection> for CodexError {
    fn from(rejection: JsonRejection) -> Self {
        let debug_message = rejection.to_string();
        match rejection {
            JsonRejection::BytesRejection(_) => CodexError::RequestRejected {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                body: OpenaiResponsesErrorObject {
                    code: Some("PAYLOAD_TOO_LARGE".to_string()),
                    message: "request body too large".to_string(),
                    r#type: "PAYLOAD_TOO_LARGE".to_string(),
                    param: None,
                },
                debug_message: Some(debug_message),
            },
            JsonRejection::JsonSyntaxError(_) => CodexError::RequestRejected {
                status: StatusCode::BAD_REQUEST,
                body: OpenaiResponsesErrorObject {
                    code: Some("INVALID_JSON".to_string()),
                    message: "invalid JSON".to_string(),
                    r#type: "INVALID_JSON".to_string(),
                    param: None,
                },
                debug_message: Some(debug_message),
            },
            _ => CodexError::RequestRejected {
                status: StatusCode::BAD_REQUEST,
                body: OpenaiResponsesErrorObject {
                    code: Some("INVALID_REQUEST".to_string()),
                    message: "invalid request".to_string(),
                    r#type: "INVALID_REQUEST".to_string(),
                    param: None,
                },
                debug_message: Some(debug_message),
            },
        }
    }
}

impl IntoResponse for CodexError {
    fn into_response(self) -> Response {
        let (status, error_body) = match self {
            CodexError::RequestRejected {
                status,
                body,
                debug_message,
            } => {
                if let Some(debug_message) = debug_message {
                    tracing::warn!(
                        status = %status,
                        code = ?body.code,
                        message = %body.message,
                        debug_message = %debug_message,
                        "Codex request rejected"
                    );
                } else {
                    tracing::warn!(
                        status = %status,
                        code = ?body.code,
                        message = %body.message,
                        "Codex request rejected"
                    );
                }

                (status, body)
            }

            CodexError::UpstreamMappedError { status, body } => {
                let cleaned = OpenaiResponsesErrorBody::from(body).inner;
                tracing::warn!(
                    status = %status,
                    code = ?cleaned.code,
                    message = %cleaned.message,
                    "Codex upstream mapped error"
                );
                (status, cleaned)
            }

            CodexError::UpstreamFallbackError { status, body } => {
                let error_body = OpenaiResponsesErrorObject {
                    code: Some(status.as_u16().to_string()),
                    message: format!("Upstream returned {status}"),
                    r#type: "UPSTREAM_ERROR".to_string(),
                    param: None,
                };
                tracing::warn!(
                    status = %status,
                    code = ?error_body.code,
                    message = %error_body.message,
                    raw_body = %format!("{:.len$}", body, len = UPSTREAM_BODY_PREVIEW_CHARS),

                    "Codex upstream fallback error"
                );
                (status, error_body)
            }

            CodexError::NoAvailableCredential => (
                StatusCode::SERVICE_UNAVAILABLE,
                OpenaiResponsesErrorObject {
                    code: Some("NO_CREDENTIAL".to_string()),
                    message: "No available credentials to process the request.".to_string(),
                    r#type: "NO_CREDENTIAL".to_string(),
                    param: None,
                },
            ),

            CodexError::Reqwest(e) => {
                tracing::warn!(error = %e, status = ?e.status(), "Codex reqwest error");
                (
                    StatusCode::BAD_GATEWAY,
                    OpenaiResponsesErrorObject {
                        code: Some("UPSTREAM_ERROR".to_string()),
                        message: "Upstream service error.".to_string(),
                        r#type: "UPSTREAM_ERROR".to_string(),
                        param: None,
                    },
                )
            }

            CodexError::StreamProtocolError(e) => {
                tracing::warn!(error = %e, "Codex stream protocol error");
                (
                    StatusCode::BAD_GATEWAY,
                    OpenaiResponsesErrorObject {
                        code: Some("UPSTREAM_ERROR".to_string()),
                        message: "Upstream stream protocol error.".to_string(),
                        r#type: "UPSTREAM_ERROR".to_string(),
                        param: None,
                    },
                )
            }

            CodexError::Internal(e) => {
                tracing::error!(error = %e, "Codex internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    OpenaiResponsesErrorObject {
                        code: Some("INTERNAL_ERROR".to_string()),
                        message: "An internal server error occurred.".to_string(),
                        r#type: "INTERNAL_ERROR".to_string(),
                        param: None,
                    },
                )
            }
        };

        let resp_json = OpenaiResponsesErrorBody { inner: error_body };
        (status, Json(resp_json)).into_response()
    }
}

impl From<crate::PolluxError> for CodexError {
    fn from(err: crate::PolluxError) -> Self {
        match err {
            crate::PolluxError::NoAvailableCredential => CodexError::NoAvailableCredential,
            crate::PolluxError::ReqwestError(e) => CodexError::Reqwest(e),
            crate::PolluxError::StreamProtocolError(s) => CodexError::StreamProtocolError(s),
            other => CodexError::Internal(other.to_string()),
        }
    }
}

impl IsRetryable for CodexError {
    fn is_retryable(&self) -> bool {
        match self {
            // Server/transport errors has been retried.
            CodexError::Reqwest(_) => false,
            CodexError::UpstreamFallbackError { status, .. } => matches!(
                *status,
                StatusCode::UNAUTHORIZED
                    | StatusCode::TOO_MANY_REQUESTS
                    | StatusCode::FORBIDDEN
                    | StatusCode::PAYMENT_REQUIRED
            ),
            CodexError::UpstreamMappedError { status, body } => {
                match *status {
                    // Only detail-matched unsupported-model 400 is retryable
                    // (credential-level model capability mismatch).
                    StatusCode::BAD_REQUEST => body.is_unsupported_model_detail(),

                    // Status-driven recoverable errors (credential refresh / cooldown / model routing).
                    StatusCode::UNAUTHORIZED
                    | StatusCode::TOO_MANY_REQUESTS
                    | StatusCode::FORBIDDEN
                    | StatusCode::PAYMENT_REQUIRED => true,

                    _ => false,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_retryable_for_mapped_bad_request_with_unsupported_model_detail() {
        let raw = r#"{"detail":"The 'gpt-5.3-codex' model is not supported when using Codex with a ChatGPT account."}"#;
        let body = serde_json::from_str::<CodexErrorBody>(raw).expect("parse sample");

        let error = CodexError::UpstreamMappedError {
            status: StatusCode::BAD_REQUEST,
            body,
        };

        assert!(error.is_retryable());
    }
}
