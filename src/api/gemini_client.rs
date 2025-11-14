use axum::{
    Json,
    body::Body,
    http::{
        HeaderValue, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING},
    },
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
};
use backon::{ExponentialBuilder, Retryable};
use chrono::{DateTime, Utc};
use eventsource_stream::{Event as UpstreamEvent, Eventsource};
use futures::{StreamExt, TryStreamExt};
use serde::Serialize;
use serde_json::{Value, json};
use std::{io, time::Duration};
use tracing::{error, info, warn};

use crate::error::NexusError;
use crate::middleware::gemini_request::{GeminiContext, GeminiRequestBody};
use crate::router::NexusState;
use crate::types::cli::{cli_bytes_to_aistudio, cli_str_to_aistudio};

use super::gemini_api::GeminiApi;

pub struct GeminiClient {
    client: reqwest::Client,
    retry_policy: ExponentialBuilder,
}

#[derive(Clone, Serialize)]
struct CliPostFormatBody {
    model: String,
    project: String,
    request: GeminiRequestBody,
}

impl GeminiClient {
    pub fn new(client: reqwest::Client) -> Self {
        let retry_policy = ExponentialBuilder::default()
            .with_min_delay(Duration::from_secs(3))
            .with_max_delay(Duration::from_secs(5))
            .with_max_times(2);
        Self {
            client,
            retry_policy,
        }
    }

    pub async fn call_gemini_cli(
        &self,
        state: &NexusState,
        ctx: &GeminiContext,
        body: &GeminiRequestBody,
    ) -> Result<reqwest::Response, NexusError> {
        let base_payload = CliPostFormatBody {
            model: ctx.model.clone(),
            project: String::new(),
            request: body.clone(),
        };

        let handle = state.handle.clone();
        let client = self.client.clone();
        let stream = ctx.stream;
        let retry_policy_inner = self.retry_policy;

        let op = {
            let base_payload = base_payload.clone();
            move || {
                let handle = handle.clone();
                let client = client.clone();
                let base_payload = base_payload.clone();
                async move {
                    let assigned =
                        handle
                            .get_credential(&ctx.model)
                            .await
                            .map_err(|e| match e {
                                NexusError::NoAvailableCredential => {
                                    error!("no available credential when getting credential");
                                    NexusError::NoAvailableCredential
                                }
                                other => {
                                    error!(error = %other, "credential acquisition failed");
                                    NexusError::CredentialAcquire(format!(
                                        "credential acquisition failed: {}",
                                        other
                                    ))
                                }
                            })?;
                    info!(
                        "Using credential ID: {} Project: {}",
                        assigned.id, assigned.project_id
                    );

                    let mut payload = base_payload.clone();
                    payload.project = assigned.project_id.clone();

                    let resp = GeminiApi::try_post_cli(
                        client.clone(),
                        assigned.access_token,
                        stream,
                        retry_policy_inner,
                        &payload,
                    )
                    .await
                    .map_err(NexusError::Reqwest)?;

                    if resp.error_for_status_ref().is_ok() {
                        return Ok(resp);
                    }

                    let status = resp.status();
                    let headers = resp.headers().clone();
                    let body = match resp.bytes().await {
                        Ok(b) => b,
                        Err(e) => {
                            warn!(error = %e, "failed to read upstream error body");
                            axum::body::Bytes::new()
                        }
                    };

                    match status {
                        StatusCode::TOO_MANY_REQUESTS => {
                            let retry_secs = parse_quota_reset_delay_secs(&body).unwrap_or(90);
                            handle
                                .report_rate_limit(
                                    assigned.id,
                                    &ctx.model,
                                    Duration::from_secs(retry_secs),
                                )
                                .await;
                            info!("Project: {}, rate limit reported", assigned.project_id);
                        }
                        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                            handle.report_invalid(assigned.id).await;
                        }
                        _ => {}
                    }

                    Err(NexusError::UpstreamHttp {
                        status,
                        headers,
                        body,
                    })
                }
            }
        };

        op.retry(&self.retry_policy)
            .when(|err: &NexusError| {
                matches!(
                    err,
                    NexusError::UpstreamHttp { status, .. }
                    if *status == StatusCode::UNAUTHORIZED || *status == StatusCode::FORBIDDEN || *status == StatusCode::TOO_MANY_REQUESTS
                )
            })
            .await
    }

    /// Map the result of `call_gemini_cli` into an `axum::Response`,
    /// converting CLI payloads into the AiStudio format (including SSE streams).
    pub async fn into_axum_response(
        ctx: &GeminiContext,
        result: Result<reqwest::Response, NexusError>,
    ) -> Response {
        match result {
            Ok(upstream_resp) => {
                if ctx.stream {
                    Self::build_stream_response(upstream_resp)
                } else {
                    Self::build_json_response(upstream_resp).await
                }
            }
            Err(NexusError::NoAvailableCredential) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "no available credential" })),
            )
                .into_response(),
            Err(NexusError::Reqwest(e)) => {
                error!(error = %e, "upstream network error after retries");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": "upstream network error" })),
                )
                    .into_response()
            }
            Err(NexusError::CredentialAcquire(msg)) => {
                error!(error = %msg, "credential acquisition failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "credential acquisition failed" })),
                )
                    .into_response()
            }
            Err(NexusError::UpstreamHttp {
                status,
                headers,
                body,
            }) => {
                let mut resp_builder = axum::response::Response::builder().status(status);
                if let Some(headers_mut) = resp_builder.headers_mut() {
                    for (k, v) in headers.iter() {
                        headers_mut.insert(k, v.clone());
                    }
                }
                resp_builder
                    .body(axum::body::Body::from(body))
                    .unwrap_or_else(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error":"build response failed"})),
                        )
                            .into_response()
                    })
            }
            Err(other) => {
                error!(error = ?other, "unhandled NexusError when calling Gemini CLI");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "internal error" })),
                )
                    .into_response()
            }
        }
    }

    async fn build_json_response(upstream_resp: reqwest::Response) -> Response {
        let status = upstream_resp.status();
        let original_headers = upstream_resp.headers().clone();
        match upstream_resp.bytes().await {
            Ok(bytes) => {
                let converted = convert_cli_envelope_bytes(&bytes);
                let content_len = converted.len();
                let mut response = Response::builder()
                    .status(status)
                    .body(Body::from(converted))
                    .unwrap_or_else(|_| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error":"build response failed"})),
                        )
                            .into_response()
                    });

                {
                    let headers_mut = response.headers_mut();
                    *headers_mut = original_headers;
                    headers_mut.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                    headers_mut.remove(CONTENT_LENGTH);
                    headers_mut.remove(TRANSFER_ENCODING);
                    if let Ok(value) = HeaderValue::from_str(&content_len.to_string()) {
                        headers_mut.insert(CONTENT_LENGTH, value);
                    }
                }

                response
            }
            Err(e) => {
                error!(error = %e, "failed to read upstream response body");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": "failed to read upstream response" })),
                )
                    .into_response()
            }
        }
    }

    fn build_stream_response(upstream_resp: reqwest::Response) -> Response {
        let status = upstream_resp.status();
        let original_headers = upstream_resp.headers().clone();
        let sse_stream = upstream_resp
            .bytes_stream()
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
            .eventsource()
            .map(|result| result.map_err(|err| io::Error::new(io::ErrorKind::Other, err)))
            .filter_map(|result| async {
                match result {
                    Ok(event) => convert_upstream_event(event).map(Ok),
                    Err(err) => Some(Err(err)),
                }
            });

        let mut response = Sse::new(sse_stream).into_response();
        *response.status_mut() = status;
        *response.headers_mut() = original_headers;
        response
    }
}

/// Parse Gemini 429 JSON body to get cooldown seconds from `quotaResetTimeStamp`.
fn parse_quota_reset_delay_secs(body: &[u8]) -> Option<u64> {
    let v: Value = serde_json::from_slice(body).ok()?;
    v.get("error")?
        .get("details")?
        .as_array()?
        .iter()
        .filter_map(|detail| {
            detail
                .get("metadata")
                .and_then(|m| m.get("quotaResetTimeStamp"))
                .and_then(|ts| ts.as_str())
                .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
        })
        .filter_map(|reset_dt| {
            let reset = reset_dt.with_timezone(&Utc);
            let now = Utc::now();
            let diff_secs = (reset - now).num_seconds();
            (diff_secs > 0).then_some(diff_secs as u64)
        })
        .next()
}

fn convert_cli_envelope_bytes(body: &[u8]) -> Vec<u8> {
    match cli_bytes_to_aistudio(body) {
        Ok(resp) => match serde_json::to_vec(&resp) {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!(error = %e, "failed to serialize converted CLI payload");
                body.to_vec()
            }
        },
        Err(e) => {
            warn!(error = %e, "failed to convert CLI response body");
            body.to_vec()
        }
    }
}

fn convert_upstream_event(event: UpstreamEvent) -> Option<Event> {
    let payload = convert_cli_sse_payload(&event.data)?;
    let mut axum_event = Event::default().data(payload);
    if !event.event.is_empty() && event.event != "message" {
        axum_event = axum_event.event(event.event);
    }
    if !event.id.is_empty() {
        axum_event = axum_event.id(event.id);
    }
    if let Some(retry) = event.retry {
        axum_event = axum_event.retry(retry);
    }
    Some(axum_event)
}

fn convert_cli_sse_payload(payload: &str) -> Option<String> {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return None;
    }
    match cli_str_to_aistudio(trimmed)
        .and_then(|resp| serde_json::to_string(&resp).map_err(Into::into))
    {
        Ok(converted) => Some(converted),
        Err(e) => {
            warn!(error = %e, "failed to parse CLI SSE payload as JSON");
            Some(trimmed.to_string())
        }
    }
}
