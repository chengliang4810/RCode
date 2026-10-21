use std::path::{Path, PathBuf};

use rcode_model::prompt_cache::SystemPromptCacheMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptSettings {
    pub cache_mode: SystemPromptCacheMode,
    pub disable_project_instructions: bool,
    pub additional_directories: Vec<PathBuf>,
}

impl PromptSettings {
    pub fn load(
        data_dir: Option<&Path>,
        options: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Self, String> {
        let mut value = match data_dir.map(|dir| dir.join("prompt-settings.json")) {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
                    .map_err(|error| format!("{}: {error}", path.display()))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
                Err(error) => return Err(format!("{}: {error}", path.display())),
            },
            None => serde_json::json!({}),
        };
        if let Some(overrides) = options.get("prompt") {
            let target = value
                .as_object_mut()
                .ok_or("prompt-settings.json must contain an object")?;
            let overrides = overrides
                .as_object()
                .ok_or("modelOptions.prompt must contain an object")?;
            target.extend(overrides.clone());
        }
        serde_json::from_value(value)
            .map_err(|error| format!("invalid main prompt settings: {error}"))
    }
}
