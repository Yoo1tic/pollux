use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::api::gemini_client::GeminiClient;
use crate::config::GEMINI_NATIVE_MODELS;
use crate::middleware::gemini_request::GeminiPreprocess;
use crate::middleware::gemini_response::{build_json_response, build_stream_response};
use crate::{NexusError, router::NexusState};

pub async fn gemini_cli_handler(
    State(state): State<NexusState>,
    GeminiPreprocess(body, ctx): GeminiPreprocess,
) -> Result<Response, NexusError> {
    // Construct caller
    let caller = GeminiClient::new(state.client.clone());

    let upstream_resp = caller.call_gemini_cli(&state, &ctx, &body).await?;

    if ctx.stream {
        Ok(build_stream_response(upstream_resp).into_response())
    } else {
        Ok(build_json_response(upstream_resp).await.into_response())
    }
}

/// Fetch Gemini native model list via API key and proxy through Nexus.
pub async fn gemini_models_handler() -> Result<Json<Value>, NexusError> {
    Ok(Json(GEMINI_NATIVE_MODELS.clone()))
}
