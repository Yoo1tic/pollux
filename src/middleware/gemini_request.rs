use axum::{
    Json, RequestExt,
    extract::{FromRequest, Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::router::NexusState;
use crate::{NexusError, api::gemini_client::GeminiClient}; // higher-level caller using the stateless GeminiApi

pub async fn gemini_cli_handler(
    State(state): State<NexusState>,
    GeminiPreprocess(body, ctx): GeminiPreprocess,
) -> Result<Response, NexusError> {
    // Construct caller
    let caller = GeminiClient::new(state.client.clone());

    let upstream_resp = caller.call_gemini_cli(&state, &ctx, &body).await?;

    if ctx.stream {
        Ok(GeminiClient::build_stream_response(upstream_resp))
    } else {
        Ok(GeminiClient::build_json_response(upstream_resp).await)
    }
}

// Move types to middleware: it is the handler layer
pub type GeminiRequestBody = serde_json::Value;

#[derive(Debug, Clone)]
pub struct GeminiContext {
    pub model: String,
    pub stream: bool,
    pub path: String,
}

pub struct GeminiPreprocess(pub GeminiRequestBody, pub GeminiContext);

impl<S> FromRequest<S> for GeminiPreprocess
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(mut req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract wildcard path as full remaining path under /models
        let Path(path) = match req.extract_parts::<Path<String>>().await {
            Ok(p) => p,
            Err(rejection) => return Err(rejection.into_response()),
        };

        // Determine model and optional rpc from the last path segment
        let last_seg = path.split('/').next_back().map(|s| s.to_string());
        let Some(last_seg) = last_seg else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "model not found in path" })),
            )
                .into_response());
        };
        let model = if let Some((m, _r)) = last_seg.split_once(':') {
            m.to_string()
        } else {
            last_seg
        };

        // Streaming decision: only `streamGenerateContent` is true; `generateContent` is false
        let stream = path.contains("streamGenerateContent");

        // Parse JSON body
        let Json(body) = match Json::<GeminiRequestBody>::from_request(req, &()).await {
            Ok(v) => v,
            Err(rejection) => return Err(rejection.into_response()),
        };

        let ctx = GeminiContext {
            model,
            stream,
            path,
        };
        Ok(GeminiPreprocess(body, ctx))
    }
}
