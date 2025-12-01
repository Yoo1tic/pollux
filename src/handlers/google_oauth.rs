use crate::config::CONFIG;
use crate::google_oauth::credentials::GoogleCredential;
use crate::google_oauth::endpoints::GoogleOauthEndpoints;
use crate::{NexusError, router::NexusState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, PrivateCookieJar, SameSite};
use base64::Engine;
use oauth2::{AuthorizationCode, CsrfToken, PkceCodeChallenge, PkceCodeVerifier};
use serde::Deserialize;
use serde_json::Value;
use subtle::ConstantTimeEq;
use time::Duration;
use tracing::info;

#[derive(Debug, Deserialize)]
pub struct AuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
}

const CSRF_COOKIE: &str = "oauth_csrf_token";
const PKCE_COOKIE: &str = "oauth_pkce_verifier";

/// GET /auth/:secret -> redirects to Google's OAuth2 consent page when the secret matches.
pub async fn google_oauth_entry(
    Path(secret): Path<String>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, NexusError> {
    if !bool::from(secret.as_bytes().ct_eq(CONFIG.nexus_key.as_bytes())) {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let pkce_verifier = verifier.secret().to_string();

    let (auth_url, csrf_token) = GoogleOauthEndpoints::build_authorize_url(challenge);

    let jar = store_oauth_cookies(jar, &csrf_token, &pkce_verifier);

    info!("Dispatching OAuth redirect");
    Ok((jar, Redirect::temporary(auth_url.as_ref())).into_response())
}

/// GET /auth/callback -> exchanges auth code for tokens and stores them.
pub async fn google_oauth_callback(
    State(state): State<NexusState>,
    Query(query): Query<AuthCallbackQuery>,
    jar: PrivateCookieJar,
) -> impl IntoResponse {
    let (pkce_verifier, csrf_cookie, jar) = match load_oauth_session(jar) {
        Ok(data) => data,
        Err((jar, err)) => return respond_with_error(jar, err),
    };

    let state_param = match query.state.as_deref() {
        Some(s) => s,
        None => {
            return respond_with_error(
                jar,
                NexusError::OauthFlowError("missing `state` in callback".to_string()),
            );
        }
    };

    if state_param != csrf_cookie {
        return respond_with_error(
            jar,
            NexusError::OauthFlowError("CSRF token mismatch".to_string()),
        );
    }

    let code = match query.code.as_deref() {
        Some(code) => code,
        None => {
            return respond_with_error(
                jar,
                NexusError::OauthFlowError("missing `code` in callback".to_string()),
            );
        }
    };

    let token_response = match GoogleOauthEndpoints::exchange_authorization_code(
        AuthorizationCode::new(code.to_owned()),
        PkceCodeVerifier::new(pkce_verifier),
        state.client.clone(),
    )
    .await
    {
        Ok(res) => res,
        Err(err) => return respond_with_error(jar, err),
    };

    let mut token_value: Value = match serde_json::to_value(&token_response) {
        Ok(v) => v,
        Err(err) => return respond_with_error(jar, NexusError::JsonError(err)),
    };
    attach_email_from_id_token(&mut token_value);

    let credential = match GoogleCredential::from_payload(&token_value) {
        Ok(cred) => cred,
        Err(err) => return respond_with_error(jar, err),
    };

    if credential.refresh_token.is_empty() {
        return respond_with_error(
            jar,
            NexusError::OauthFlowError(
                "OAuth response missing refresh_token; ensure access_type=offline and prompt=consent are allowed for this client/user".to_string(),
            ),
        );
    }
    if credential.access_token.is_none() {
        return respond_with_error(
            jar,
            NexusError::UnexpectedError("missing access_token in OAuth response".into()),
        );
    }

    state
        .handle
        .submit_credentials(vec![credential.clone()])
        .await;

    info!("OAuth callback stored credential");
    (jar, Json(credential)).into_response()
}

fn attach_email_from_id_token(token_value: &mut Value) {
    let Some(id_token) = token_value.get("id_token").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(payload_b64) = id_token.split('.').nth(1) else {
        return;
    };
    let Some(decoded) = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .ok()
    else {
        return;
    };
    let Ok(payload_json) = serde_json::from_slice::<Value>(&decoded) else {
        return;
    };
    let Some(email) = payload_json.get("email").and_then(|e| e.as_str()) else {
        return;
    };
    if let Some(obj) = token_value.as_object_mut() {
        obj.insert("email".to_string(), Value::String(email.to_string()));
    }
}

fn store_oauth_cookies(
    jar: PrivateCookieJar,
    csrf: &CsrfToken,
    pkce_verifier: &str,
) -> PrivateCookieJar {
    jar.add(build_cookie(CSRF_COOKIE, csrf.secret().to_string()))
        .add(build_cookie(PKCE_COOKIE, pkce_verifier.to_string()))
}

fn load_oauth_session(
    jar: PrivateCookieJar,
) -> Result<(String, String, PrivateCookieJar), (PrivateCookieJar, NexusError)> {
    let Some(csrf_cookie) = jar.get(CSRF_COOKIE).map(|c| c.value().to_owned()) else {
        let jar = clear_oauth_cookies(jar);
        return Err((
            jar,
            NexusError::OauthFlowError("Missing CSRF token in cookie".to_string()),
        ));
    };

    let Some(pkce_cookie) = jar.get(PKCE_COOKIE).map(|c| c.value().to_owned()) else {
        let jar = clear_oauth_cookies(jar);
        return Err((
            jar,
            NexusError::OauthFlowError("Missing PKCE verifier in cookie".to_string()),
        ));
    };

    let jar = clear_oauth_cookies(jar);

    Ok((pkce_cookie, csrf_cookie, jar))
}

fn clear_oauth_cookies(jar: PrivateCookieJar) -> PrivateCookieJar {
    jar.remove(clear_cookie(CSRF_COOKIE))
        .remove(clear_cookie(PKCE_COOKIE))
}

fn build_cookie(name: &str, value: String) -> Cookie<'static> {
    Cookie::build(Cookie::new(name.to_string(), value))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(Duration::minutes(15))
        .build()
}

fn clear_cookie(name: &str) -> Cookie<'static> {
    Cookie::build(Cookie::new(name.to_string(), ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .build()
}

fn respond_with_error(jar: PrivateCookieJar, err: NexusError) -> Response {
    (jar, err.into_response()).into_response()
}
