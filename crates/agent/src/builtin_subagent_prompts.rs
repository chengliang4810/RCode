//! English role definitions ported from the local ZCode reference.
//! Source revision, adaptations and hashes: prompts/subagents/source.json.

pub const GENERAL_PURPOSE_DESCRIPTION: &str = "General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks. When you are searching for a keyword or file and are not confident that you will find the right match in the first few tries use this agent to perform the search for you.";
pub const GENERAL_PURPOSE_SUMMARY: &str = "General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks.";
pub const GENERAL_PURPOSE_PROMPT: &str = include_str!("prompts/subagents/general-purpose.md");

pub const EXPLORE_DESCRIPTION: &str = "Read-only search agent for broad fan-out searches - when answering means sweeping many files, directories, or naming conventions and you only need the conclusion, not the file dumps. It reads excerpts rather than whole files, so it locates code; it doesn't review or audit it. Specify search breadth: \"medium\" for moderate exploration, \"very thorough\" for multiple locations and naming conventions.";
pub const EXPLORE_SUMMARY: &str = "Read-only search agent for broad fan-out searches.";
pub const EXPLORE_PROMPT: &str = include_str!("prompts/subagents/explore.md");

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    fn role_text_matches_the_zcode_snapshot() {
        let snapshot: serde_json::Value =
            serde_json::from_str(include_str!("prompts/subagents/source.json")).unwrap();
        for (name, description, prompt) in [
            (
                "general-purpose",
                GENERAL_PURPOSE_DESCRIPTION,
                GENERAL_PURPOSE_PROMPT,
            ),
            ("explore", EXPLORE_DESCRIPTION, EXPLORE_PROMPT),
        ] {
            let role = snapshot["roles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|role| role["name"] == name)
                .unwrap();
            assert_eq!(description, role["description"].as_str().unwrap());
            assert_eq!(
                format!("{:x}", Sha256::digest(prompt.as_bytes())),
                role["sha256"].as_str().unwrap()
            );
        }
    }
}
