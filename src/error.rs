use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
};
use sqlx::Error as SqlxError;
use thiserror::Error as ThisError;

// Shorten long oauth2 type paths used below.
use oauth2::basic::BasicErrorResponseType;
use oauth2::reqwest::Error as ReqwestClientError;
use oauth2::{HttpClientError, RequestTokenError, StandardErrorResponse};

#[derive(Debug, ThisError)]
pub enum NexusError {
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("HTTP request error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Missing access token; refresh first")]
    MissingAccessToken,

    #[error("missing email in userinfo response")]
    MissingEmailInUserinfo,

    #[error("OAuth2 token request error: {0}")]
    Oauth2Token(String),

    #[error("OAuth2 server error: {error}")]
    Oauth2Server { error: String },

    #[error("no available credential in queue")]
    NoAvailableCredential,

    #[error("Ractor error: {0}")]
    RactorError(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] SqlxError),

    #[error("credential acquisition failed: {0}")]
    CredentialAcquire(String),

    #[error("upstream HTTP error: {status}")]
    UpstreamHttp {
        status: StatusCode,
        headers: HeaderMap,
        body: Bytes,
    },
}

impl NexusError {}

impl
    From<
        RequestTokenError<
            HttpClientError<ReqwestClientError>,
            StandardErrorResponse<BasicErrorResponseType>,
        >,
    > for NexusError
{
    fn from(
        e: RequestTokenError<
            HttpClientError<ReqwestClientError>,
            StandardErrorResponse<BasicErrorResponseType>,
        >,
    ) -> Self {
        match e {
            RequestTokenError::ServerResponse(err) => NexusError::Oauth2Server {
                error: err.error().to_string(),
            },
            RequestTokenError::Request(req_e) => {
                NexusError::Oauth2Token(format!("request failed: {}", req_e))
            }
            RequestTokenError::Parse(parse_err, _body) => NexusError::Json(parse_err.into_inner()),
            RequestTokenError::Other(s) => NexusError::Oauth2Token(s),
        }
    }
}
