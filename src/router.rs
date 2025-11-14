use axum::{Router, middleware, routing::any};

use crate::config::{CLI_USER_AGENT, CONFIG};
use crate::service::credentials_actor::CredentialsHandle;

#[derive(Clone)]
pub struct NexusState {
    pub handle: CredentialsHandle,
    pub client: reqwest::Client,
}

impl NexusState {
    pub fn new(handle: CredentialsHandle) -> Self {
        let mut builder = reqwest::Client::builder().user_agent(CLI_USER_AGENT.as_str());
        if let Some(proxy_url) = CONFIG.proxy.clone() {
            let proxy = reqwest::Proxy::all(proxy_url.as_str())
                .expect("invalid PROXY url for reqwest client");
            builder = builder.proxy(proxy);
        }
        let client = builder
            .build()
            .expect("failed to build reqwest client for proxy");
        Self { handle, client }
    }
}

pub fn nexus_router(state: NexusState) -> Router {
    use crate::middleware::auth::RequireKeyAuth;
    use crate::middleware::gemini_request::gemini_cli_handler;
    Router::new()
        .route("/v1beta/models/{*path}", any(gemini_cli_handler))
        .layer(middleware::from_extractor::<RequireKeyAuth>())
        .with_state(state)
}
