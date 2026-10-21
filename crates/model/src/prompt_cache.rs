// Modified for RCode.
//! Provider-neutral transport contract for preserving a system prompt cache seam.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SystemPromptCacheMode {
    #[default]
    Organization,
    Global,
    ToolBased,
}

/// Sections retain their identity until the provider constructs its wire body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPrompt {
    pub client_header: Option<String>,
    pub identity: String,
    pub static_sections: Vec<String>,
    #[serde(default)]
    pub frozen_sections: Vec<String>,
    pub dynamic_sections: Vec<String>,
    pub cache_mode: SystemPromptCacheMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPromptCacheScope {
    Organization,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPromptBlock {
    pub text: String,
    pub cache_scope: Option<SystemPromptCacheScope>,
}

impl SystemPrompt {
    pub fn blocks(&self) -> Vec<SystemPromptBlock> {
        let mut blocks = Vec::new();
        let mut push = |text: String, cache_scope| {
            let text = strip_system_prompt_dynamic_boundaries(&text);
            if !text.trim().is_empty() {
                blocks.push(SystemPromptBlock { text, cache_scope });
            }
        };
        if let Some(header) = &self.client_header {
            push(header.clone(), None);
        }
        let global = self.cache_mode == SystemPromptCacheMode::Global;
        push(
            self.identity.clone(),
            (!global).then_some(SystemPromptCacheScope::Organization),
        );
        if global {
            push(
                self.static_sections.join("\n\n"),
                Some(SystemPromptCacheScope::Global),
            );
            push(
                self.frozen_sections.join("\n\n"),
                Some(SystemPromptCacheScope::Organization),
            );
        } else {
            push(
                self.static_sections
                    .iter()
                    .chain(&self.frozen_sections)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                Some(SystemPromptCacheScope::Organization),
            );
        }
        push(self.dynamic_sections.join("\n\n"), None);
        blocks
    }

    pub fn text(&self) -> String {
        self.blocks()
            .into_iter()
            .map(|block| block.text)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Reserved zero-width transport token between cacheable and dynamic system sections.
///
/// Producers may place this token in the model-neutral system text. Provider adapters
/// must consume it before constructing their wire request.
pub const SYSTEM_PROMPT_DYNAMIC_BOUNDARY: &str = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";

/// Removes every reserved cache-boundary token without changing surrounding bytes.
pub fn strip_system_prompt_dynamic_boundaries(text: &str) -> String {
    text.replace(SYSTEM_PROMPT_DYNAMIC_BOUNDARY, "")
}

/// Combines an optional frozen system prompt with a request-time contribution.
///
/// A non-empty contribution is always placed after an explicit cache boundary.
/// Removing the reserved token therefore reproduces the bytes emitted by the
/// legacy `base + "\n\n" + dynamic` composition, including its empty/absent-base
/// edge cases.
pub fn combine_system_prompt_with_dynamic(base: Option<&str>, dynamic: &str) -> Option<String> {
    if dynamic.is_empty() {
        return base.map(str::to_owned);
    }

    match base {
        Some(base) if base.matches(SYSTEM_PROMPT_DYNAMIC_BOUNDARY).count() == 1 => {
            Some(format!("{base}\n\n{dynamic}"))
        }
        Some(base) => Some(format!(
            "{base}{SYSTEM_PROMPT_DYNAMIC_BOUNDARY}\n\n{dynamic}"
        )),
        None => Some(format!("{SYSTEM_PROMPT_DYNAMIC_BOUNDARY}{dynamic}")),
    }
}
