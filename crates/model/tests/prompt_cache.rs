//! Provider wire stability under equivalent tool discovery orders.
use rcode_model::{
    AnthropicConfig, AnthropicModel, JsonObject, Model, ModelMessage, ModelRequest,
    ModelRuntimeConfig, OpenAiConfig, OpenAiModel, OpenAiResponsesConfig, OpenAiResponsesModel,
    ToolDefinition,
};
use serde_json::json;
use url::Url;

#[test]
fn tool_discovery_order_does_not_change_any_provider_request_or_tool_contract() {
    let endpoint = Url::parse("https://example.invalid/v1").unwrap();
    let runtime = ModelRuntimeConfig::with_full_observation();
    let models: Vec<Box<dyn Model>> = vec![
        Box::new(OpenAiModel::new(
            OpenAiConfig::new(endpoint.clone(), "fixture", "model").with_runtime(runtime.clone()),
        )),
        Box::new(AnthropicModel::new(
            AnthropicConfig::new(endpoint.clone(), "fixture", "model")
                .with_runtime(runtime.clone()),
        )),
        Box::new(OpenAiResponsesModel::new(
            OpenAiResponsesConfig::new(endpoint, "fixture", "model").with_runtime(runtime),
        )),
    ];
    let tools = ["zeta_read", "alpha_read", "mcp__files__read"]
        .into_iter()
        .map(|name| {
            ToolDefinition::new(
                name,
                JsonObject::from_value(json!({
                    "type": "object", "properties": {"path": {"type": "string"}},
                    "required": ["path"], "additionalProperties": false,
                }))
                .unwrap(),
            )
            .with_description(format!("Read with {name}."))
        })
        .collect::<Vec<_>>();
    let request = ModelRequest::new(vec![ModelMessage::user_text("Read the requested file.")])
        .with_tools(tools.clone());
    let mut reordered = request.clone();
    reordered.tools.reverse();
    for model in models {
        let first = model.prepare_request(&request).unwrap();
        let second = model.prepare_request(&reordered).unwrap();
        assert_eq!(first.body().as_value(), second.body().as_value());
        let wire_tools = first.body().as_value()["tools"].as_array().unwrap();
        assert_eq!(wire_tools.len(), tools.len());
        let mut names = Vec::new();
        for tool in wire_tools {
            let definition = tool.get("function").unwrap_or(tool);
            let name = definition["name"].as_str().unwrap();
            names.push(name);
            let original = tools.iter().find(|tool| tool.name == name).unwrap();
            assert_eq!(
                definition["description"],
                original.description.as_deref().unwrap()
            );
            let schema = definition
                .get("input_schema")
                .or_else(|| definition.get("parameters"))
                .unwrap();
            assert_eq!(
                schema,
                &serde_json::to_value(&original.input_schema).unwrap()
            );
        }
        assert_eq!(names, ["alpha_read", "mcp__files__read", "zeta_read"]);
    }
    assert_eq!(
        request.tools, tools,
        "serialization must not reorder the caller's registry"
    );
}
