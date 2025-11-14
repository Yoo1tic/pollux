use serde::Deserialize;
use serde_json::Value;

use super::aistudio::{Candidate as AiCandidate, Chat, FinishReason, GeminiResponse};

/// Generic CLI envelope wrapper.
#[derive(Debug, Deserialize)]
pub struct CliResponseEnvelope<T> {
    pub response: T,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct CliResponse {
    pub candidates: Vec<CliCandidate>,
    pub usageMetadata: Value,
    pub modelVersion: String,
    #[serde(default)]
    pub promptFeedback: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct CliCandidate {
    pub content: Chat,
    #[serde(default)]
    pub finishReason: Option<FinishReason>,
}

impl From<CliResponse> for GeminiResponse {
    fn from(value: CliResponse) -> Self {
        let candidates = value
            .candidates
            .into_iter()
            .map(CliCandidate::into_ai)
            .collect();
        GeminiResponse {
            candidates,
            usageMetadata: value.usageMetadata,
            modelVersion: value.modelVersion,
            promptFeedback: value.promptFeedback,
        }
    }
}

impl CliCandidate {
    fn into_ai(self) -> AiCandidate {
        AiCandidate {
            content: self.content,
            finishReason: self.finishReason,
        }
    }
}

fn try_parse_envelope_bytes(body: &[u8]) -> Result<CliResponse, serde_json::Error> {
    let envelope: CliResponseEnvelope<CliResponse> = serde_json::from_slice(body)?;
    Ok(envelope.response)
}

fn try_parse_envelope_str(payload: &str) -> Result<CliResponse, serde_json::Error> {
    let envelope: CliResponseEnvelope<CliResponse> = serde_json::from_str(payload)?;
    Ok(envelope.response)
}

/// Parse raw CLI JSON bytes into the AiStudio-compatible response struct.
pub fn cli_bytes_to_aistudio(body: &[u8]) -> Result<GeminiResponse, serde_json::Error> {
    match try_parse_envelope_bytes(body) {
        Ok(resp) => Ok(resp.into()),
        Err(_) => serde_json::from_slice::<CliResponse>(body).map(Into::into),
    }
}

/// Parse raw CLI JSON string into the AiStudio-compatible response struct.
pub fn cli_str_to_aistudio(payload: &str) -> Result<GeminiResponse, serde_json::Error> {
    match try_parse_envelope_str(payload) {
        Ok(resp) => Ok(resp.into()),
        Err(_) => serde_json::from_str::<CliResponse>(payload).map(Into::into),
    }
}
