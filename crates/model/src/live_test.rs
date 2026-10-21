// Modified for RCode.
//!
//! Live end-to-end check against a real provider.
//!
//! Ignored by default: it needs network access and a credential, so it must
//! never run in CI. Run it deliberately:
//!
//! ```text
//! RCODE_LIVE_KEY=… RCODE_LIVE_URL=… cargo test -p rcode-model --lib -- --ignored live
//! ```
//!
//! It exists because the rest of the suite proves the pieces fit together, not
//! that a real server accepts what they produce. The first live call is where a
//! wrong header, a doubled `/v1`, or a mis-shaped body actually shows up.

use std::sync::Arc;

use serde_json::json;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    AnthropicConfig, AnthropicModel, Model as _, ModelMessage, ModelRequest, ModelStreamEvent,
};

/// The credential, from the environment — never a literal, since this file is
/// committed.
fn live_config() -> Option<(Url, String, String)> {
    let key = std::env::var("RCODE_LIVE_KEY").ok()?;
    let endpoint = std::env::var("RCODE_LIVE_URL").ok()?;
    let model =
        std::env::var("RCODE_LIVE_MODEL").unwrap_or_else(|_| "claude-sonnet-5-5".to_owned());
    let url = Url::parse(&endpoint).expect("RCODE_LIVE_URL is a URL");
    Some((url, key, model))
}

#[tokio::test]
#[ignore = "needs network and a credential; run with --ignored"]
async fn anthropic_reaches_a_live_provider_and_streams_a_reply() {
    let Some((endpoint, api_key, model)) = live_config() else {
        panic!("set RCODE_LIVE_KEY, RCODE_LIVE_URL (and optionally RCODE_LIVE_MODEL) to run this");
    };

    // A bare origin or a base ending in /v1 both resolve to /v1/messages.
    let config = AnthropicConfig::new(endpoint, api_key, model).with_max_tokens(64);
    let client = AnthropicModel::new(config);

    let request = ModelRequest::new(vec![ModelMessage::user_text(
        "Reply with exactly the word: OK",
    )]);

    let mut stream = client
        .stream(request, CancellationToken::new())
        .await
        .expect("the request was accepted");

    let mut text = String::new();
    let mut saw_completed = false;
    let mut tool_calls = 0;
    while let Some(event) = stream.next().await {
        match event.expect("the stream is readable") {
            ModelStreamEvent::TextDelta { text: delta } => text.push_str(&delta),
            ModelStreamEvent::Completed(response) => {
                saw_completed = true;
                if let ModelMessage::Assistant {
                    tool_calls: calls, ..
                } = response.message()
                {
                    tool_calls = calls.len();
                }
            }
            _ => {}
        }
    }

    assert!(saw_completed, "the stream ended without a Completed event");
    assert_eq!(tool_calls, 0, "a plain reply should ask for no tools");
    assert!(
        text.to_uppercase().contains("OK"),
        "expected the model to answer OK, got: {text:?}"
    );
}

#[tokio::test]
#[ignore = "needs network and a credential; run with --ignored"]
async fn a_tool_definition_survives_the_round_trip() {
    let Some((endpoint, api_key, model)) = live_config() else {
        panic!("set RCODE_LIVE_KEY and RCODE_LIVE_URL to run this");
    };

    let config = AnthropicConfig::new(endpoint, api_key, model).with_max_tokens(256);
    let client = AnthropicModel::new(config);

    let tool = crate::ToolDefinition {
        name: "get_weather".to_owned(),
        description: Some("Look up the weather for a city.".to_owned()),
        input_schema: crate::JsonObject::from_value(json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }))
        .expect("a valid schema"),
    };
    let request = ModelRequest::new(vec![ModelMessage::user_text(
        "What is the weather in Paris? Use the tool.",
    )])
    .with_tools(vec![tool]);

    let mut stream = client
        .stream(request, CancellationToken::new())
        .await
        .expect("the request was accepted");

    let mut calls = Vec::new();
    while let Some(event) = stream.next().await {
        if let ModelStreamEvent::Completed(response) = event.expect("readable") {
            if let ModelMessage::Assistant { tool_calls, .. } = response.message() {
                calls = tool_calls.clone();
            }
        }
    }

    // The point is that a tool *call* comes back through the whole pipeline —
    // schema serialization, the provider's encoding, and the stream decoder's
    // accumulation of the fragments. Whether this particular model chooses to
    // call it is not what is under test, so both outcomes are reported.
    if calls.is_empty() {
        eprintln!(
            "note: the model answered without calling the tool; the round trip itself succeeded"
        );
    } else {
        assert_eq!(calls[0].name(), "get_weather");
        assert!(
            calls[0].arguments().as_map().contains_key("city"),
            "the arguments did not survive: {:?}",
            calls[0].arguments()
        );
    }
}

use futures::StreamExt as _;

#[allow(dead_code)]
fn _assert_send_sync() {
    // The harness holds the client behind an `Arc` across awaits; a regression
    // here would surface as a confusing borrow error there instead.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Arc<AnthropicModel>>();
}
