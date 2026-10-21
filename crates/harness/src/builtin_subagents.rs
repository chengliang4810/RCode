//! Built-in roles for the native agent.
use rcode_agent::builtin_subagent_prompts::*;
use rcode_agent::subagents::{ModelAlias, SubagentDefinition};

pub(crate) fn with_builtins(mut definitions: Vec<SubagentDefinition>) -> Vec<SubagentDefinition> {
    for (name, description, model, tools, prompt) in [
        (
            "general-purpose",
            GENERAL_PURPOSE_DESCRIPTION,
            ModelAlias::Inherit,
            &[][..],
            GENERAL_PURPOSE_PROMPT,
        ),
        (
            "Explore",
            EXPLORE_DESCRIPTION,
            ModelAlias::Inherit,
            &[
                "Bash",
                "Glob",
                "Grep",
                "Read",
                "WebFetch",
                "WebSearch",
                "TodoWrite",
            ][..],
            EXPLORE_PROMPT,
        ),
    ] {
        if definitions.iter().any(|definition| definition.name == name) {
            continue;
        }
        definitions.push(SubagentDefinition {
            name: name.into(),
            description: description.into(),
            model,
            tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
            disallowed_tools: vec!["Agent".into()],
            system_prompt: prompt.into(),
            path: format!("builtin:{name}").into(),
            source: rcode_agent::skills::SkillSource::User,
        });
    }
    definitions
}
