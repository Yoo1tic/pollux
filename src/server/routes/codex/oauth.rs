use crate::PolluxError;
use crate::error::OauthError;
use crate::providers::codex::client::oauth::endpoints::CodexOauthEndpoints;
use crate::providers::codex::oauth::OauthTokenResponse;
use crate::server::router::PolluxState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::cookie::{Cookie, PrivateCookieJar, SameSite};
use oauth2::{AuthorizationCode, PkceCodeChallenge, PkceCodeVerifier, TokenResponse};
use serde::Deserialize;
use time::Duration;
use tracing::{error, info};

const CSRF_COOKIE: &str = "codex_oauth_csrf_token";
const PKCE_COOKIE: &str = "codex_oauth_pkce_verifier";

#[derive(Debug, Deserialize)]
pub struct AuthCallbackQuery {
    pub code: String,
    pub state: String,
}

/// GET /codex/auth
///
/// Starts the Codex OAuth2 PKCE flow and redirects the browser to the OpenAI auth page.
pub async fn codex_oauth_entry(
    State(state): State<PolluxState>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, PolluxError> {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_token) = CodexOauthEndpoints::build_authorize_url(challenge);

    let jar = jar
        .add(build_cookie(
            CSRF_COOKIE,
            csrf_token.secret().to_string(),
            !state.insecure_cookie,
        ))
        .add(build_cookie(
            PKCE_COOKIE,
            verifier.secret().to_string(),
            !state.insecure_cookie,
        ));

    info!("Dispatching Codex OAuth redirect to: {}", auth_url);
    Ok((jar, Redirect::temporary(auth_url.as_ref())).into_response())
}

/// GET /codex/auth/callback
pub async fn codex_oauth_callback(
    State(state): State<PolluxState>,
    Query(query): Query<AuthCallbackQuery>,
    jar: PrivateCookieJar,
) -> impl IntoResponse {
    let (jar, session_data) = take_oauth_cookies(jar);

    let result = process_oauth_exchange(&state, &query.code, &query.state, session_data).await;
    match result {
        Ok(token_response) => {
            // Hand off to the Codex actor for identity extraction + persistence + activation.
            state
                .providers
                .codex
                .submit_trusted_oauth(token_response)
                .await;
            info!("Codex OAuth callback accepted");
            (jar, (StatusCode::ACCEPTED, "Success")).into_response()
        }
        Err(err) => {
            error!("Codex OAuth failure: {:?}", err);
            (jar, err.into_response()).into_response()
        }
    }
}

async fn process_oauth_exchange(
    state: &PolluxState,
    code: &str,
    state_param: &str,
    session_data: Option<(String, String)>,
) -> Result<OauthTokenResponse, PolluxError> {
    let (pkce_verifier, csrf_token) = session_data.ok_or_else(|| OauthError::Flow {
        code: "OAUTH_SESSION_MISSING".to_string(),
        message: "Missing OAuth session cookies".to_string(),
        details: None,
    })?;

    if state_param != csrf_token {
        return Err(OauthError::Flow {
            code: "CSRF_MISMATCH".to_string(),
            message: "CSRF token mismatch".to_string(),
            details: None,
        }
        .into());
    }

    let token_response: OauthTokenResponse = CodexOauthEndpoints::exchange_authorization_code(
        AuthorizationCode::new(code.to_string()),
        PkceCodeVerifier::new(pkce_verifier),
        state.codex_client.clone(),
    )
    .await
    .map_err(|e| OauthError::Flow {
        code: "TOKEN_EXCHANGE_FAILED".to_string(),
        message: format!("Token exchange failed: {e}"),
        details: None,
    })?;

    if token_response.refresh_token().is_none() {
        return Err(OauthError::Flow {
            code: "MISSING_REFRESH_TOKEN".to_string(),
            message: "Missing refresh_token (check offline_access)".to_string(),
            details: None,
        }
        .into());
    }

    if token_response
        .extra_fields()
        .id_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        return Err(OauthError::Flow {
            code: "MISSING_ID_TOKEN".to_string(),
            message: "Missing id_token in token response".to_string(),
            details: None,
        }
        .into());
    }

    Ok(token_response)
}

fn take_oauth_cookies(jar: PrivateCookieJar) -> (PrivateCookieJar, Option<(String, String)>) {
    let csrf = jar.get(CSRF_COOKIE).map(|c| c.value().to_string());
    let pkce = jar.get(PKCE_COOKIE).map(|c| c.value().to_string());

    let jar = jar
        .remove(Cookie::from(CSRF_COOKIE))
        .remove(Cookie::from(PKCE_COOKIE));

    match (pkce, csrf) {
        (Some(p), Some(c)) => (jar, Some((p, c))),
        _ => (jar, None),
    }
}

fn build_cookie(name: &'static str, value: String, secure: bool) -> Cookie<'static> {
    Cookie::build((name, value))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(Duration::minutes(15))
        .build()
}
