//! The application's display language, for the parts of the UI that speak.
//!
//! A seam, deliberately thin. The plan calls for the language to be read once
//! at startup — after settings are loaded, before any window or native menu is
//! built — and then handed to whatever needs it. `Localizer` is that value;
//! this module is where the *one* instance lives so a render path does not have
//! to thread it through a dozen call sites, and so there is exactly one place
//! to change when the startup order changes.
//!
//! The preference itself is not read here yet: the settings page that stores it
//! is not built, so this resolves against the system locale. When that lands,
//! the initializer takes the stored preference and nothing else moves.
//!
//! A process-wide value for the *language* is not the same as a global for the
//! *strings*: the strings are looked up through the returned `Localizer`, which
//! a test can construct for any language it likes without touching this.

use std::sync::OnceLock;

use rcode_presentation::{LanguagePreference, Localizer};

/// The language every window shares.
///
/// Resolved once. Changing it takes a restart by design — the alternative is
/// windows disagreeing mid-session about which language they are in, which is
/// worse than a restart prompt.
///
/// A caller that never calls [`init`] gets the system language, which is what
/// a test and any path that runs before settings are read should see.
pub fn localizer() -> &'static Localizer {
    LOCALIZER.get_or_init(|| Localizer::new(LanguagePreference::System))
}

static LOCALIZER: OnceLock<Localizer> = OnceLock::new();

/// Fix the language for this process.
///
/// Called once, after settings are read and before any window or native menu
/// exists — a window built first would render its title in the previous
/// language and never be corrected. A second call is ignored rather than
/// switching mid-flight: the choice takes effect on restart, and silently
/// changing it here would leave already-drawn surfaces in the old language.
pub fn init(preference: LanguagePreference) {
    let _ = LOCALIZER.set(Localizer::new(preference));
}

/// The question pages the wizard renders.
pub fn question_pages(
    request: &rcode_proto::InteractionRequest,
) -> Vec<rcode_proto::UserInputQuestion> {
    let rcode_proto::InteractionRequest::Questions { questions } = request;
    questions.clone()
}

/// Keep question answers typed across the input bridge.
pub fn response_for(
    _request: &rcode_proto::InteractionRequest,
    answers: Vec<rcode_proto::UserInputAnswer>,
) -> rcode_proto::InteractionResponse {
    rcode_proto::InteractionResponse::Questions { answers }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_request_passes_its_pages_and_answers_through() {
        let request = rcode_proto::InteractionRequest::Questions {
            questions: vec![rcode_proto::UserInputQuestion {
                id: "q0".to_owned(),
                header: "H".to_owned(),
                question: "Q?".to_owned(),
                options: Vec::new(),
                multi_select: false,
                prefill: None,
                multiline: true,
            }],
        };
        assert_eq!(question_pages(&request).len(), 1);
        let answers = vec![rcode_proto::UserInputAnswer {
            question_id: "q0".to_owned(),
            selected_option_ids: Vec::new(),
            text: Some("typed".to_owned()),
        }];
        assert_eq!(
            response_for(&request, answers.clone()),
            rcode_proto::InteractionResponse::Questions { answers }
        );
    }
}
