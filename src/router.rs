use crate::config::{CLI_USER_AGENT, CONFIG, COOKIE_KEY};
use crate::handlers::oauth_flow::{google_oauth_callback, google_oauth_entry};
use crate::service::credentials_actor::CredentialsHandle;
use axum::{
    Router,
    extract::FromRef,
    middleware,
    routing::{get, post},
};
use axum_extra::extract::cookie::Key;
use reqwest::header::{CONNECTION, HeaderMap, HeaderValue};
use std::time::Duration;

#[derive(Clone)]
pub struct NexusState {
    pub handle: CredentialsHandle,
    pub client: reqwest::Client,
}

impl NexusState {
    pub fn new(handle: CredentialsHandle) -> Self {
        let mut headers = HeaderMap::new();

        let mut builder = reqwest::Client::builder()
            .user_agent(CLI_USER_AGENT.as_str())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(10 * 60));

        if let Some(proxy_url) = CONFIG.proxy.clone() {
            let proxy = reqwest::Proxy::all(proxy_url.as_str())
                .expect("invalid PROXY url for reqwest client");
            builder = builder.proxy(proxy);
        }

        if !CONFIG.enable_multiplexing {
            headers.insert(CONNECTION, HeaderValue::from_static("close"));

            builder = builder
                .http1_only()
                .pool_max_idle_per_host(0)
                .pool_idle_timeout(Duration::from_secs(0));
        } else {
            builder = builder.http2_adaptive_window(true);
        }

        let client = builder
            .default_headers(headers)
            .build()
            .expect("failed to build reqwest client for proxy");

        Self { handle, client }
    }
}

impl FromRef<NexusState> for Key {
    fn from_ref(state: &NexusState) -> Self {
        let _ = state; // state not used to fetch the static key
        COOKIE_KEY.clone()
    }
}

pub fn nexus_router(state: NexusState) -> Router {
    use crate::handlers::gemini::{
        gemini_cli_handler, gemini_models_handler, openai_models_handler,
    };
    use crate::middleware::auth::RequireKeyAuth;

    let authed = Router::new()
        .route("/v1beta/models", get(gemini_models_handler))
        .route("/v1beta/openai/models", get(openai_models_handler))
        .route("/v1beta/models/{*path}", post(gemini_cli_handler))
        .layer(middleware::from_extractor::<RequireKeyAuth>());

    let oauth = Router::new()
        .route("/auth/{secret}", get(google_oauth_entry))
        .route("/oauth2callback", get(google_oauth_callback));

    Router::new().merge(oauth).merge(authed).with_state(state)
}
