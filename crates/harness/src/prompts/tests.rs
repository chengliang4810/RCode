use super::*;
use rcode_model::prompt_cache::{SystemPromptCacheMode, SystemPromptCacheScope};

async fn prepare(
    sessions: &PromptSessions,
    cwd: &Path,
    tools: &ToolRegistry,
    settings: &PromptSettings,
    memory: &str,
    mcp: Option<&str>,
    id: &str,
) -> PreparedPrompt {
    sessions
        .prepare(PromptRequest {
            cwd,
            data_dir: None,
            user_home: None,
            tools,
            memory: Some(memory),
            mcp_instructions: mcp.map(str::to_owned),
            settings,
            resume: Some(id),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn default_base_contains_only_the_approved_text_and_environment() {
    let project = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(project.path())
            .status()
            .unwrap()
            .success()
    );
    let sessions = PromptSessions::default();
    let mut tools = ToolRegistry::new();
    rcode_agent::register_core_tools(&mut tools);
    for registry in [&ToolRegistry::new(), &tools] {
        let result = sessions
            .prepare(PromptRequest {
                cwd: project.path(),
                data_dir: None,
                user_home: None,
                tools: registry,
                memory: None,
                mcp_instructions: None,
                settings: &PromptSettings::default(),
                resume: None,
            })
            .await
            .unwrap();
        let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(
            normalize(&result.system.static_sections.join("\n\n")),
            normalize(MAIN_SYSTEM_PROMPT)
        );
        assert_eq!(result.system.identity, IDENTITY);
        assert!(result.system.dynamic_sections.is_empty());
        assert_eq!(result.system.frozen_sections.len(), 2);
        let environment = &result.system.frozen_sections[0];
        let fields: Vec<_> = environment
            .lines()
            .filter_map(|line| line.strip_prefix("- "))
            .map(|line| line.split_once(": ").unwrap().0)
            .collect();
        assert_eq!(
            fields,
            [
                "Primary working directory",
                "Is a git repository",
                "Additional working directories",
                "Platform",
                "Shell used by Bash",
            ]
        );
        assert!(environment.contains("Is a git repository: true"));
        assert!(!environment.contains("test-model"));
    }
}

#[tokio::test]
async fn project_sections_are_cached_and_memory_refreshes_between_runs() {
    let project = tempfile::tempdir().unwrap();
    let file = project.path().join("AGENTS.md");
    std::fs::write(&file, "First project instruction").unwrap();
    let sessions = PromptSessions::default();
    let tools = ToolRegistry::new();
    let settings = PromptSettings::default();
    let first = prepare(
        &sessions,
        project.path(),
        &tools,
        &settings,
        "old memory",
        None,
        "one",
    )
    .await;
    std::fs::write(&file, "Updated project instruction").unwrap();
    let second = prepare(
        &sessions,
        project.path(),
        &tools,
        &settings,
        "new memory",
        None,
        "one",
    )
    .await;
    assert!(first.system.text().contains("old memory"));
    assert!(second.system.text().contains("new memory"));
    assert!(!second.system.text().contains("old memory"));
    assert!(second.system.text().contains("First project instruction"));
    let other = prepare(
        &sessions,
        project.path(),
        &tools,
        &settings,
        "new memory",
        None,
        "two",
    )
    .await;
    assert!(other.system.text().contains("Updated project instruction"));
    let reset = prepare(
        &sessions,
        project.path(),
        &tools,
        &settings,
        "new memory",
        None,
        "one",
    )
    .await;
    assert!(reset.system.text().contains("new memory"));
    assert!(reset.system.text().contains("First project instruction"));
    assert!(!reset.system.text().contains("project-instructions"));
    assert!(!reset.system.text().contains("Instructions from"));
}

#[test]
fn removed_prompt_overrides_are_rejected_from_files_and_request_options() {
    let dir = tempfile::tempdir().unwrap();
    for field in [
        "systemPrompt",
        "appendSystemPrompt",
        "overrideSystemPrompt",
        "agentSystemPrompt",
        "coordinatorSystemPrompt",
        "includeMemoryWithCustomPrompt",
    ] {
        let value = serde_json::json!({field: "REMOVED PROMPT"});
        std::fs::write(dir.path().join("prompt-settings.json"), value.to_string()).unwrap();
        assert!(
            PromptSettings::load(Some(dir.path()), &Default::default()).is_err(),
            "removed file setting was accepted: {field}"
        );
        let options = serde_json::json!({"prompt": value});
        assert!(
            PromptSettings::load(None, options.as_object().unwrap()).is_err(),
            "removed request setting was accepted: {field}"
        );
    }
}

#[tokio::test]
async fn optional_context_is_appended_in_order_after_the_complete_base() {
    let project = tempfile::tempdir().unwrap();
    let sessions = PromptSessions::default();
    let mut tools = ToolRegistry::new();
    rcode_agent::register_core_tools(&mut tools);
    tools.register(Arc::new(McpFixture));
    let settings = PromptSettings::default();
    let result = prepare(
        &sessions,
        project.path(),
        &tools,
        &settings,
        "MEMORY BODY",
        Some("MCP BODY"),
        "one",
    )
    .await;
    let text = result.system.text();
    assert!(text.contains("# Doing tasks"));
    let sections = [
        "# Communication",
        "# Formatting rules",
        "# Environment",
        "MEMORY BODY",
        "# MCP server instructions\n\nMCP BODY",
    ];
    for pair in sections.windows(2) {
        assert!(text.find(pair[0]).unwrap() < text.find(pair[1]).unwrap());
    }
    assert_eq!(result.system.dynamic_sections.len(), 2);
    assert!(!text.contains("# Language"));
    assert!(!text.contains("# Output Style"));
}

#[tokio::test]
async fn final_cache_blocks_follow_all_three_reference_modes() {
    let project = tempfile::tempdir().unwrap();
    let sessions = PromptSessions::default();
    for mode in [
        SystemPromptCacheMode::Organization,
        SystemPromptCacheMode::Global,
        SystemPromptCacheMode::ToolBased,
    ] {
        let settings = PromptSettings {
            cache_mode: mode,
            ..Default::default()
        };
        let result = prepare(
            &sessions,
            project.path(),
            &ToolRegistry::new(),
            &settings,
            "",
            None,
            "one",
        )
        .await;
        let blocks = result.system.blocks();
        assert_eq!(
            blocks.len(),
            if mode == SystemPromptCacheMode::Global {
                3
            } else {
                2
            }
        );
        assert!(result.system.client_header.is_none());
        assert_eq!(blocks[0].text, IDENTITY);
        if mode == SystemPromptCacheMode::Global {
            assert_eq!(blocks[0].cache_scope, None);
            assert_eq!(blocks[1].cache_scope, Some(SystemPromptCacheScope::Global));
            assert_eq!(
                blocks[2].cache_scope,
                Some(SystemPromptCacheScope::Organization)
            );
        } else {
            assert_eq!(
                blocks[0].cache_scope,
                Some(SystemPromptCacheScope::Organization)
            );
            assert_eq!(
                blocks[1].cache_scope,
                Some(SystemPromptCacheScope::Organization)
            );
        }
    }
}

#[test]
fn invalid_settings_fail_instead_of_silently_changing_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("prompt-settings.json"), "{broken").unwrap();
    assert!(PromptSettings::load(Some(dir.path()), &Default::default()).is_err());
}

struct McpFixture;

#[async_trait::async_trait]
impl rcode_agent::Tool for McpFixture {
    fn name(&self) -> &str {
        "mcp__fixture__read"
    }
    fn description(&self) -> &str {
        "Fixture"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    async fn call(
        &self,
        _: serde_json::Value,
        _: &rcode_agent::ToolContext,
    ) -> Result<rcode_agent::ToolResult, rcode_agent::ToolError> {
        unreachable!()
    }
}

#[tokio::test]
async fn mcp_and_memory_refresh_while_static_sections_are_cached() {
    let project = tempfile::tempdir().unwrap();
    let sessions = PromptSessions::default();
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(McpFixture));
    let settings = PromptSettings::default();
    prepare(
        &sessions,
        project.path(),
        &tools,
        &settings,
        "OLD MEMORY",
        Some("FIRST MCP"),
        "one",
    )
    .await;
    let next = prepare(
        &sessions,
        project.path(),
        &tools,
        &settings,
        "NEW MEMORY",
        Some("SECOND MCP"),
        "one",
    )
    .await
    .system
    .text();
    assert!(next.contains("SECOND MCP"));
    assert!(!next.contains("FIRST MCP"));
    assert!(!next.contains("OLD MEMORY"));
    assert!(next.contains("NEW MEMORY"));
    let disconnected = prepare(
        &sessions,
        project.path(),
        &ToolRegistry::new(),
        &settings,
        "OLD MEMORY",
        Some("SECOND MCP"),
        "one",
    )
    .await
    .system
    .text();
    assert!(!disconnected.contains("SECOND MCP"));
    assert!(disconnected.contains("OLD MEMORY"));
    let disabled = sessions
        .prepare(PromptRequest {
            cwd: project.path(),
            data_dir: None,
            user_home: None,
            tools: &ToolRegistry::new(),
            memory: None,
            mcp_instructions: None,
            settings: &settings,
            resume: Some("one"),
        })
        .await
        .unwrap();
    assert!(!disabled.system.text().contains("OLD MEMORY"));
}
