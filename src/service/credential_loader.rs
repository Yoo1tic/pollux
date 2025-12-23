use crate::error::NexusError;
use crate::google_oauth::credentials::GoogleCredential;
use serde_json::Value;
use std::{fs, path::Path};
use tracing::{info, warn};

/// Load credential JSON files from a directory into GoogleCredential structs.
pub fn load_from_dir(dir: &Path) -> Result<Vec<GoogleCredential>, NexusError> {
    if !dir.exists() {
        info!(path = %dir.display(), "credentials directory not found; skipping load");
        return Ok(Vec::new());
    }

    let loaded: Vec<GoogleCredential> = fs::read_dir(dir)?
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry.path()),
            Err(e) => {
                let err: NexusError = e.into();
                warn!(error = %err, "failed to read credentials dir entry");
                None
            }
        })
        .filter(|path| is_json_file(path))
        .filter_map(|path| {
            load_credential(&path)
                .inspect_err(|e| {
                    warn!(path = %path.display(), error = %e, "failed to load credential");
                })
                .ok()
        })
        .collect();

    Ok(loaded)
}

fn is_json_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json"))
        == Some(true)
}

fn load_credential(path: &Path) -> Result<GoogleCredential, NexusError> {
    let contents = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&contents)?;
    GoogleCredential::from_payload(&value)
}
