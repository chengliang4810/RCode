//! `AskUserQuestion` — the model asks the user something.
//!
//! This tool does not answer anything itself. It validates the questions and
//! hands them back through the loop, which waits on the user input bridge. The answer arrives as this tool's result, so the model
//! reads it like any other tool output.
//!
//! Because the loop owns that bridge, the tool's `call` never runs — the runner
//! recognises it by name and takes a different path. The implementation here
//! exists so the schema, the description and the validation have a home, and so
//! a caller that invokes it directly gets a usable error instead of silence.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolError, ToolResult};

/// The tool's name, which the loop matches on to take the asking path.
pub const ASK_USER_QUESTION: &str = "AskUserQuestion";

/// Questions accepted in one call.
const MAX_QUESTIONS: usize = 4;
/// Choices accepted per question.
const MAX_OPTIONS: usize = 10;

pub struct AskUserQuestionTool;

#[async_trait]
impl Tool for AskUserQuestionTool {
    fn name(&self) -> &str {
        ASK_USER_QUESTION
    }

    fn description(&self) -> &str {
        "Ask the user a question and wait for their answer. Use it when the \
         decision is theirs — a preference, an ambiguous requirement, a choice \
         between approaches with different trade-offs — rather than something \
         you can settle from the code. Put question text on each question and \
         provide options as strings or objects with label and optional description."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "One to four questions.",
                    "minItems": 1,
                    "maxItems": MAX_QUESTIONS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Short identifier, unique in this call."
                            },
                            "header": {
                                "type": "string",
                                "description": "A few words naming the decision, e.g. `Storage`."
                            },
                            "question": {
                                "type": "string",
                                "description": "The question, ending in a question mark."
                            },
                            "options": {
                                "type": "array",
                                "items": {
                                    "anyOf": [
                                        { "type": "string", "minLength": 1 },
                                        {
                                            "type": "object",
                                            "properties": {
                                                "label": { "type": "string", "minLength": 1 },
                                                "description": { "type": "string" }
                                            },
                                            "required": ["label"]
                                        }
                                    ]
                                },
                                "maxItems": MAX_OPTIONS,
                                "description": "Choices as strings or {label, description} objects. The user picks one unless multiSelect is set. For free text, omit options and set multiline to true."
                            },
                            "multiSelect": {
                                "type": "boolean",
                                "description": "Allow more than one choice."
                            },
                            "prefill": {
                                "type": "string",
                                "description": "A suggested answer the user can edit."
                            },
                            "multiline": {
                                "type": "boolean",
                                "description": "Accept a free-form answer instead of only the choices."
                            }
                        },
                        "required": ["question"]
                    }
                }
            },
            "required": ["questions"]
        })
    }

    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // Reached only if something invokes the tool directly. The loop
        // intercepts it by name, so in a normal run this never executes.
        let questions = validate(&input)?;
        Err(ToolError::Execution(format!(
            "{} must be handled by the agent loop, which asks the user; it was \
             invoked directly with {} question(s)",
            ASK_USER_QUESTION, questions
        )))
    }
}

/// Check the payload and return how many questions it holds.
///
/// Shared with the loop so the schema and the enforcement cannot drift.
pub fn validate(input: &Value) -> Result<usize, ToolError> {
    let questions = input
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ToolError::InvalidInput("`questions` is required and must be an array".to_owned())
        })?;
    if questions.is_empty() {
        return Err(ToolError::InvalidInput(
            "`questions` is empty; ask at least one".to_owned(),
        ));
    }
    if questions.len() > MAX_QUESTIONS {
        return Err(ToolError::InvalidInput(format!(
            "{} questions is more than the {MAX_QUESTIONS} allowed in one call",
            questions.len()
        )));
    }

    for (index, question) in questions.iter().enumerate() {
        let text = question_text(question).ok_or_else(|| {
            ToolError::InvalidInput(format!("question {index} has no `question` text"))
        })?;
        if let Some(options) = question.get("options") {
            let options = options.as_array().ok_or_else(|| {
                ToolError::InvalidInput(format!("question {index}: `options` must be an array"))
            })?;
            if options.len() > MAX_OPTIONS {
                return Err(ToolError::InvalidInput(format!(
                    "question {index} offers {} choices, over the {MAX_OPTIONS} allowed",
                    options.len()
                )));
            }
            for (position, option) in options.iter().enumerate() {
                if option_label(option).is_none() {
                    return Err(ToolError::InvalidInput(format!(
                        "question {index}, option {position}: provide a non-empty string or an object with a non-empty `label` (and optional `description`)"
                    )));
                }
            }
        }
        // A free-form question is fine; one with neither options nor multiline
        // is not answerable, which would hang the run.
        let has_options = question
            .get("options")
            .and_then(Value::as_array)
            .is_some_and(|options| !options.is_empty());
        let multiline = question
            .get("multiline")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !has_options && !multiline {
            return Err(ToolError::InvalidInput(format!(
                "question {index} (`{text}`) has no choices and is not multiline, so it \
                 cannot be answered; give `options` or set `multiline: true`"
            )));
        }
    }

    Ok(questions.len())
}

fn nonempty_text(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// Preserve explanatory text in the shared protocol's option label.
fn option_label(option: &Value) -> Option<String> {
    if let Some(text) = nonempty_text(option) {
        return Some(text.to_owned());
    }
    let label = option
        .get("label")
        .and_then(nonempty_text)
        .or_else(|| option.get("prefill").and_then(nonempty_text));
    let description = option.get("description").and_then(nonempty_text);
    match (label, description) {
        (Some(label), Some(description)) if label != description => {
            Some(format!("{label} — {description}"))
        }
        (Some(text), _) | (None, Some(text)) => Some(text.to_owned()),
        _ => None,
    }
}

fn question_text(question: &Value) -> Option<&str> {
    if let Some(text) = question.get("question").and_then(nonempty_text) {
        return Some(text);
    }
    // Recover the observed misplaced prompt only when every option names the
    // same question. Never invent a prompt or choose between conflicts.
    let options = question.get("options")?.as_array()?;
    let prompt = options.first()?.get("question").and_then(nonempty_text)?;
    options
        .iter()
        .all(|option| option.get("question").and_then(nonempty_text) == Some(prompt))
        .then_some(prompt)
}

/// Turn model strings and object choices into the host's typed questions.
/// Option ids are generated by position, so identical labels remain distinct.
///
/// This is the decoder the loop's `InputRequest::Questions` leaves to the tool
/// (see the module note beside it): the tool is the only place that knows what
/// it asked for, so it is the only place that can say what an answer means.
///
/// Assumes [`validate`] has passed. It reads defensively anyway — a payload
/// that changed shape between the two calls should drop a question, not panic.
pub fn decode(input: &Value) -> Vec<rcode_proto::UserInputQuestion> {
    let Some(entries) = input.get("questions").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let prompt = question_text(entry)?.to_owned();
            let options = entry
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .enumerate()
                        .filter_map(|(position, option)| {
                            let label = option_label(option)?;
                            Some(rcode_proto::QuestionOption {
                                // Positional and stable: the model re-sends the
                                // same list for the same question, so the ids
                                // match what the answer refers to.
                                id: format!("{index}:{position}"),
                                label,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(rcode_proto::UserInputQuestion {
                id: format!("q{index}"),
                header: entry
                    .get("header")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                question: prompt,
                options,
                multi_select: entry
                    .get("multiSelect")
                    .or_else(|| entry.get("multi_select"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                prefill: entry
                    .get("prefill")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                multiline: entry
                    .get("multiline")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// Render typed answers for the model, mapping option ids back to their labels.
///
/// The model asked in words and must be answered in words: an id like `0:1`
/// means nothing to it. The mapping is the inverse of [`decode`], and lives
/// beside it so the two cannot disagree about what an id names.
pub fn render_answers(
    questions: &[rcode_proto::UserInputQuestion],
    answers: &[rcode_proto::UserInputAnswer],
) -> String {
    let label_for = |question_id: &str, option_id: &str| -> Option<&str> {
        questions
            .iter()
            .find(|question| question.id == question_id)?
            .options
            .iter()
            .find(|option| option.id == option_id)
            .map(|option| option.label.as_str())
    };

    let mut lines = Vec::new();
    for answer in answers {
        let Some(question) = questions.iter().find(|q| q.id == answer.question_id) else {
            // An answer to a question nobody asked: skip it rather than
            // inventing a prompt for it.
            continue;
        };
        let labels: Vec<&str> = answer
            .selected_option_ids
            .iter()
            .filter_map(|id| label_for(&answer.question_id, id))
            .collect();
        let mut line = question.question.clone();
        let mut parts = Vec::new();
        if !labels.is_empty() {
            parts.push(labels.join(", "));
        }
        if let Some(text) = answer
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            parts.push(text.to_owned());
        }
        if parts.is_empty() {
            // Answering nothing is a valid thing for a user to do, and the
            // model needs to be told rather than shown a blank.
            line.push_str("\n  → (no answer)");
        } else {
            line.push_str(&format!("\n  → {}", parts.join(" — ")));
        }
        lines.push(line);
    }

    if lines.is_empty() {
        return "The user did not answer.".to_owned();
    }
    lines.join("\n")
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    #[test]
    fn object_choices_preserve_labels_descriptions_and_multi_select() {
        let payload = json!({"questions":[{"question":"Which?", "multiSelect":true,
            "options":[{"label":"First", "description":"Keep existing data"},
                       {"description":"Second"}, "Third"]}]});
        assert_eq!(validate(&payload).unwrap(), 1);
        let questions = decode(&payload);
        assert!(questions[0].multi_select);
        assert_eq!(questions[0].options[0].label, "First — Keep existing data");
        assert_eq!(questions[0].options[1].label, "Second");
        assert_eq!(questions[0].options[2].label, "Third");
        let rendered = render_answers(
            &questions,
            &[rcode_proto::UserInputAnswer {
                question_id: "q0".into(),
                selected_option_ids: vec!["0:0".into(), "0:1".into()],
                text: None,
            }],
        );
        assert!(rendered.contains("First — Keep existing data, Second"));
        let legacy =
            json!({"questions":[{"question":"Which?", "multi_select":true, "options":["a"]}]});
        assert!(decode(&legacy)[0].multi_select);
    }

    #[test]
    fn misplaced_prompt_is_recovered_only_when_all_options_agree() {
        let mut payload = json!({"questions":[{"options":[
            {"question":"Which?", "prefill":"First", "description":"Keep data"},
            {"question":"Which?", "prefill":"Second", "description":"Remove data"}
        ]}]});
        assert_eq!(validate(&payload).unwrap(), 1);
        let questions = decode(&payload);
        assert_eq!(questions[0].question, "Which?");
        assert_eq!(questions[0].options[0].label, "First — Keep data");
        payload["questions"][0]["options"][1]["question"] = json!("Something else?");
        assert!(validate(&payload).is_err());
        assert!(decode(&payload).is_empty());
    }

    #[test]
    fn empty_or_non_text_choices_are_rejected_instead_of_waiting_for_input() {
        for option in [
            json!(" "),
            json!({"label":" ", "description":""}),
            json!({}),
            json!(1),
            json!(null),
        ] {
            let payload = json!({"questions":[{"question":"Which?", "options":[option]}]});
            assert!(validate(&payload).is_err(), "{payload}");
        }
    }

    #[test]
    fn options_become_positional_ids() {
        // The model gives strings; the protocol needs ids. Position is the
        // only stable thing to derive them from.
        let payload = json!({ "questions": [{ "question": "Which?", "options": ["a", "b"] }] });
        let questions = decode(&payload);
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].id, "q0");
        assert_eq!(questions[0].options[0].id, "0:0");
        assert_eq!(questions[0].options[0].label, "a");
        assert_eq!(questions[0].options[1].id, "0:1");
    }

    #[test]
    fn two_identical_labels_still_get_different_ids() {
        // The reason ids exist: a reword or a translation must not change which
        // option came back.
        let payload = json!({ "questions": [{ "question": "Sure?", "options": ["Yes", "Yes"] }] });
        let questions = decode(&payload);
        assert_eq!(questions[0].options[0].label, questions[0].options[1].label);
        assert_ne!(questions[0].options[0].id, questions[0].options[1].id);
    }

    #[test]
    fn a_question_without_options_decodes_to_a_free_form_question() {
        let payload = json!({ "questions": [{ "question": "How?", "multiline": true }] });
        let questions = decode(&payload);
        assert_eq!(questions.len(), 1);
        assert!(questions[0].options.is_empty());
        assert!(questions[0].multiline);
    }

    #[test]
    fn an_answer_renders_as_the_labels_the_model_asked_with() {
        // Ids mean nothing to the model; it must be answered in words.
        let payload =
            json!({ "questions": [{ "question": "Which?", "options": ["alpha", "beta"] }] });
        let questions = decode(&payload);
        let answers = vec![rcode_proto::UserInputAnswer {
            question_id: "q0".to_owned(),
            selected_option_ids: vec!["0:1".to_owned()],
            text: None,
        }];
        let rendered = render_answers(&questions, &answers);
        assert!(rendered.contains("Which?"), "got: {rendered}");
        assert!(
            rendered.contains("beta"),
            "the label should appear: {rendered}"
        );
        assert!(
            !rendered.contains("0:1"),
            "an id must not reach the model: {rendered}"
        );
    }

    #[test]
    fn free_text_rides_alongside_a_choice() {
        let payload = json!({ "questions": [{ "question": "Which?", "options": ["alpha"] }] });
        let questions = decode(&payload);
        let answers = vec![rcode_proto::UserInputAnswer {
            question_id: "q0".to_owned(),
            selected_option_ids: vec!["0:0".to_owned()],
            text: Some("but see the note".to_owned()),
        }];
        let rendered = render_answers(&questions, &answers);
        assert!(rendered.contains("alpha"));
        assert!(rendered.contains("but see the note"));
    }

    #[test]
    fn an_empty_answer_says_so_rather_than_rendering_blank() {
        // "picked nothing" is a valid answer and the model must be told;
        // an empty string would read as a rendering bug.
        let payload = json!({ "questions": [{ "question": "Which?", "options": ["a"] }] });
        let questions = decode(&payload);
        let answers = vec![rcode_proto::UserInputAnswer {
            question_id: "q0".to_owned(),
            selected_option_ids: Vec::new(),
            text: None,
        }];
        let rendered = render_answers(&questions, &answers);
        assert!(rendered.contains("no answer"), "got: {rendered}");
    }

    #[test]
    fn an_answer_with_no_questions_at_all_says_so() {
        let rendered = render_answers(&[], &[]);
        assert!(rendered.contains("did not answer"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn context() -> ToolContext {
        ToolContext {
            cwd: std::env::temp_dir(),
            cancel: CancellationToken::new(),
            nested: None,
        }
    }

    fn question(text: &str) -> Value {
        json!({ "question": text, "options": ["a", "b"] })
    }

    #[test]
    fn a_well_formed_payload_validates() {
        let payload = json!({ "questions": [question("Which?"), question("And?")] });
        assert_eq!(validate(&payload).expect("valid"), 2);
    }

    #[test]
    fn a_missing_questions_key_is_refused() {
        assert!(validate(&json!({})).is_err());
    }

    #[test]
    fn an_empty_list_is_refused() {
        let error = validate(&json!({ "questions": [] })).expect_err("empty");
        assert!(error.to_string().contains("at least one"));
    }

    #[test]
    fn too_many_questions_are_refused() {
        let questions: Vec<Value> = (0..MAX_QUESTIONS + 1)
            .map(|index| question(&format!("q{index}")))
            .collect();
        let error = validate(&json!({ "questions": questions })).expect_err("too many");
        assert!(error.to_string().contains("more than"));
    }

    #[test]
    fn a_question_without_text_is_refused() {
        let error = validate(&json!({ "questions": [{ "options": ["a"] }] })).expect_err("no text");
        assert!(error.to_string().contains("no `question` text"));
    }

    #[test]
    fn too_many_options_are_refused() {
        let options: Vec<&str> = (0..MAX_OPTIONS + 1).map(|_| "x").collect();
        let error = validate(&json!({ "questions": [{ "question": "q", "options": options }] }))
            .expect_err("too many options");
        assert!(error.to_string().contains("over the"));
    }

    #[test]
    fn a_question_that_cannot_be_answered_is_refused() {
        // Neither choices nor multiline: the panel would have nothing to show,
        // and the run would sit waiting forever.
        let error =
            validate(&json!({ "questions": [{ "question": "q" }] })).expect_err("unanswerable");
        assert!(error.to_string().contains("cannot be answered"));
    }

    #[test]
    fn a_multiline_question_needs_no_options() {
        let payload = json!({ "questions": [{ "question": "q", "multiline": true }] });
        assert!(validate(&payload).is_ok());
    }

    #[test]
    fn the_name_is_the_one_the_loop_matches() {
        assert_eq!(AskUserQuestionTool.name(), ASK_USER_QUESTION);
    }

    #[tokio::test]
    async fn calling_it_directly_reports_the_misuse() {
        let error = AskUserQuestionTool
            .call(json!({ "questions": [question("Which?")] }), &context())
            .await
            .expect_err("direct call");
        assert!(error.to_string().contains("agent loop"));
    }
}
