use crate::config::CONFIG;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiModel {
    name: String,
    version: Option<String>,
    display_name: String,
    description: Option<String>,
    input_token_limit: Option<u64>,
    output_token_limit: Option<u64>,
    supported_generation_methods: Option<Vec<String>>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    top_k: Option<u64>,
    max_temperature: Option<f64>,
    thinking: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GeminiModelList {
    models: Vec<GeminiModel>,
}

/// OpenAI-compatible model entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAIModel {
    pub id: String,
    pub object: String,
    pub owned_by: String,
    pub display_name: String,
}

/// OpenAI-compatible model list wrapper.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAIModelList {
    pub object: String,
    pub data: Vec<OpenAIModel>,
}

impl GeminiModel {
    fn from_name(name: String) -> Self {
        Self {
            name: name.clone(),
            version: None,
            display_name: name,
            description: None,
            input_token_limit: None,
            output_token_limit: None,
            supported_generation_methods: None,
            temperature: None,
            top_p: None,
            top_k: None,
            max_temperature: None,
            thinking: None,
        }
    }
}

pub static GEMINI_NATIVE_MODELS: LazyLock<GeminiModelList> = LazyLock::new(|| {
    let models = CONFIG
        .model_list
        .iter()
        .cloned()
        .map(GeminiModel::from_name)
        .collect();

    GeminiModelList { models }
});

pub static GEMINI_OAI_MODELS: LazyLock<OpenAIModelList> = LazyLock::new(|| {
    let data = GEMINI_NATIVE_MODELS
        .models
        .iter()
        .map(|m| OpenAIModel {
            id: m.name.clone(),
            object: "model".to_string(),
            owned_by: "google".to_string(),
            display_name: m.display_name.clone(),
        })
        .collect();

    OpenAIModelList {
        object: "list".to_string(),
        data,
    }
});
