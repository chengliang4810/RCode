//! The native run protocol: the types a run is made of.
//!
//! Everything that crosses between the loop, the host and the engine belongs
//! here, so there is exactly one definition of each. Before this module the
//! question and tool types lived in several crates. Shared types keep the
//! loop, host, storage and UI in step.
//!
//! # What belongs here
//!
//! Only data: no behaviour that needs a model, a filesystem or a window. The
//! loop, the host and the engine each keep their own logic and share these
//! shapes.
//!
//! # What does *not*
//!
//! The provider wire format (`rcode-model`) and the loop's internal message
//! type (`rcode-agent`) are deliberately separate: a provider message carries
//! protocol detail no one else should see, and a loop message carries
//! execution state the wire has no use for. Merging them would make every
//! provider change a protocol change.

use serde::{Deserialize, Serialize};

// ─── Questions ──────────────────────────────────────────────────────────────

/// One choice a question offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    /// Stable within the question. **Not** the label: two options may read the
    /// same and still be different choices, and the answer comes back as an id
    /// so a translation or a reword cannot silently change which was picked.
    pub id: String,
    /// What the user reads.
    pub label: String,
}

impl From<&str> for QuestionOption {
    /// An option whose id is its label.
    ///
    /// For a caller with no reason to distinguish two identical labels — a
    /// fixture, or a stub panel. Real questions generate positional ids
    /// (`ask_user::decode`), because there the distinction matters.
    fn from(label: &str) -> Self {
        Self {
            id: label.to_owned(),
            label: label.to_owned(),
        }
    }
}

impl From<String> for QuestionOption {
    fn from(label: String) -> Self {
        Self {
            id: label.clone(),
            label,
        }
    }
}

// ─── Interaction ────────────────────────────────────────────────────────────

/// Something the run is waiting on the user for.
///
/// The engine parks the run until an answer, dismissal or cancellation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InteractionRequest {
    Questions {
        questions: Vec<crate::agent::UserInputQuestion>,
    },
}

/// What came back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InteractionResponse {
    Questions {
        answers: Vec<crate::agent::UserInputAnswer>,
    },
    /// The user closed the prompt without answering.
    ///
    /// Distinct from submitting an empty answer: the question was cancelled.
    Dismissed,
}

// ─── Tool data ──────────────────────────────────────────────────────────────

/// A tool call, as the agent made it.
///
/// The execution-side shape. [`crate::ToolCall`] is its *presentation*
/// counterpart: what the transcript renders. Keeping them apart matters
/// because the display one is lossy by design (arguments are summarized,
/// large payloads are stripped) and must never be fed back to a tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInvocation {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// A tool call's result, as the agent got it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutput {
    pub output: String,
    pub is_error: bool,
    /// A structured diff, when the tool made a file change it can describe.
    ///
    /// Structured rather than a string so the layer that knows the before and
    /// after states produces it, and the layer that renders it does not have
    /// to parse anything back out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<Diff>,
}

/// One file's change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub path: String,
    /// `None` means the file is new.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    pub new_text: String,
}

// ─── Identities ─────────────────────────────────────────────────────────────

/// The identities a run is addressed by, named so they cannot be confused.
///
/// Each is a different thing and several are strings on the wire, which is
/// exactly how one gets passed where another is meant. Naming them here is
/// documentation the compiler can enforce at the boundary:
///
/// - [`RunId`] — one accepted run.
/// - [`MessageId`] — one entry in the chat document.
/// - [`ModelMessageId`] — one message a model call produced.
/// - [`InteractionId`] — one request the user must answer.
///
/// The rule that follows from the split: a completed model step carries a
/// [`ModelMessageId`] and must never be mistaken for a document entry, or one
/// tool call ends up split across two entries in the transcript.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelMessageId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InteractionId(pub String);

// ─── Errors ─────────────────────────────────────────────────────────────────

/// An error with a stable code, so a display layer can localize it.
///
/// The alternative — shipping an English sentence — puts the wording in the
/// data, where it cannot be translated and cannot be matched on. A caller that
/// wants to branch on the *kind* of failure uses [`Self::code`]; a caller that
/// wants to show it looks the code up and passes [`Self::args`].
///
/// `detail` is the raw text from whatever failed — a provider message, a
/// command's stderr. It is shown as technical detail and never translated,
/// because it is someone else's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunError {
    /// Stable, language-independent, and part of the contract. Changing one is
    /// a compatibility break, so they are kebab-case and specific
    /// (`provider-key-missing`, not `config-error`).
    pub code: String,
    /// Values the message needs. Never secrets, and never a whole provider
    /// response — a log line may carry this.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub args: std::collections::BTreeMap<String, String>,
    /// The underlying text, untranslated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl RunError {
    /// An error carrying only a code.
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            args: Default::default(),
            detail: None,
        }
    }

    /// Add a value the message needs.
    pub fn with_arg(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.args.insert(key.into(), value.into());
        self
    }

    /// Attach the underlying text.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_permission_settings_are_ignored_and_not_written_back() {
        for mode in [
            "default",
            "acceptEdit",
            "autoMode",
            "bypass",
            "unknown-future-mode",
        ] {
            let config: crate::ChatConfig = serde_json::from_value(serde_json::json!({
                "harness": "native", "model": "model", "reasoning": null,
                "permissionMode": mode
            }))
            .expect("existing chat configuration remains readable");
            assert_eq!(config.model.as_deref(), Some("model"));
            assert!(
                serde_json::to_value(config)
                    .unwrap()
                    .get("permissionMode")
                    .is_none()
            );

            let request: crate::RunRequest = serde_json::from_value(serde_json::json!({
                "prompt": "go", "cwd": ".", "resume": null,
                "permissionMode": mode, "sandbox": "workspace-write", "autoApprove": false
            }))
            .expect("old run requests remain readable");
            let json = serde_json::to_value(request).unwrap();
            for retired in ["permissionMode", "sandbox", "autoApprove"] {
                assert!(json.get(retired).is_none());
            }
        }
    }

    #[test]
    fn questions_answers_and_dismissal_keep_their_wire_shapes() {
        let request_json = serde_json::json!({"kind": "questions", "questions": [{
            "id": "q1", "header": "Approach", "question": "Which?",
            "options": [{"id": "a", "label": "First"}],
            "multiSelect": false, "multiline": false
        }]});
        let request: InteractionRequest = serde_json::from_value(request_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), request_json);
        let answer_json = serde_json::json!({"kind": "questions", "answers": [{
            "questionId": "q1", "selectedOptionIds": ["a"], "text": "details"
        }]});
        let answer: InteractionResponse = serde_json::from_value(answer_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(answer).unwrap(), answer_json);
        assert_eq!(
            serde_json::to_value(InteractionResponse::Dismissed).unwrap(),
            serde_json::json!({"kind": "dismissed"})
        );
    }

    #[test]
    fn two_options_may_read_the_same_and_stay_distinct() {
        // The reason an answer is an id: a translation or a reword must not
        // change which choice came back.
        let options = vec![
            QuestionOption {
                id: "a".to_owned(),
                label: "Yes".to_owned(),
            },
            QuestionOption {
                id: "b".to_owned(),
                label: "Yes".to_owned(),
            },
        ];
        assert_eq!(options[0].label, options[1].label);
        assert_ne!(options[0].id, options[1].id);
    }

    #[test]
    fn an_empty_answer_is_not_a_dismissal() {
        // "typed nothing" and "closed the dialog" are different: one is an
        // answer, the other grants nothing.
        let answered = InteractionResponse::Questions {
            answers: vec![crate::agent::UserInputAnswer {
                question_id: "q1".to_owned(),
                selected_option_ids: Vec::new(),
                text: None,
            }],
        };
        assert!(!matches!(answered, InteractionResponse::Dismissed));
    }

    #[test]
    fn a_run_error_serializes_its_code_and_args_without_a_rendered_sentence() {
        let error = RunError::new("provider-key-missing")
            .with_arg("provider", "openai")
            .with_detail("401 unauthorized");
        let value = serde_json::to_value(&error).expect("serialize");
        assert_eq!(value["code"], "provider-key-missing");
        assert_eq!(value["args"]["provider"], "openai");
        assert_eq!(value["detail"], "401 unauthorized");
    }

    #[test]
    fn a_run_error_without_args_omits_the_field() {
        let value = serde_json::to_value(RunError::new("engine-unreachable")).expect("serialize");
        assert!(value.get("args").is_none());
        assert!(value.get("detail").is_none());
    }

    #[test]
    fn a_tool_output_carries_its_diff_as_structure() {
        // The tool layer knows the before and after; making the event layer
        // re-derive a diff from text is how `diff: None` got hardcoded.
        let output = ToolOutput {
            output: "edited".to_owned(),
            is_error: false,
            diff: Some(Diff {
                path: "src/main.rs".to_owned(),
                old_text: Some("let x = 1;".to_owned()),
                new_text: "let x = 2;".to_owned(),
            }),
        };
        let value = serde_json::to_value(&output).expect("serialize");
        assert_eq!(value["diff"]["path"], "src/main.rs");
        assert_eq!(value["diff"]["oldText"], "let x = 1;");
    }

    #[test]
    fn a_new_file_has_no_old_text() {
        let diff = Diff {
            path: "new.rs".to_owned(),
            old_text: None,
            new_text: "fn main() {}".to_owned(),
        };
        let value = serde_json::to_value(&diff).expect("serialize");
        assert!(value.get("oldText").is_none(), "a new file has no before");
    }

    #[test]
    fn the_identities_are_things_apart() {
        // They are all strings on the wire, which is how one gets passed where
        // another is meant. Distinct types make that a compile error.
        fn takes_run(_: RunId) {}
        takes_run(RunId("r".to_owned()));
        // The following would not compile, which is the point:
        // takes_run(MessageId("m".to_owned()));
    }
}
