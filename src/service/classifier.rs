use std::collections::HashSet;

/// Classifies Gemini model names as "big" or not.
pub trait ModelClassifier: Send + Sync {
    fn is_big_model(&self, model_name: &str) -> bool;
}

/// Simple classifier built from a statically configured allow-list.
pub struct BigModelList {
    models: HashSet<String>,
}

impl BigModelList {
    pub fn new(big_models: Vec<String>) -> Self {
        Self {
            models: big_models.into_iter().collect(),
        }
    }
}

impl ModelClassifier for BigModelList {
    fn is_big_model(&self, model_name: &str) -> bool {
        self.models.contains(model_name)
    }
}
