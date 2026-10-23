//! Native host failures are typed before they become display copy.
use rcode_presentation::{Localizer, TextKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BrowserError {
    Message(TextKey),
    #[cfg(any(target_os = "linux", test))]
    WithReason(TextKey, String),
    /// Raw diagnostics supplied by the operating system or WebView runtime.
    Native(String),
}

impl BrowserError {
    pub fn display_message(&self) -> String {
        self.localized(crate::l10n::localizer())
    }

    fn localized(&self, localizer: &Localizer) -> String {
        match self {
            Self::Message(key) => localizer.text(*key),
            #[cfg(any(target_os = "linux", test))]
            Self::WithReason(key, reason) => localizer.format_with(*key, &[("reason", reason)]),
            Self::Native(reason) => {
                localizer.format_with(TextKey::BrowserOpenFailed, &[("reason", reason)])
            }
        }
    }
}

impl From<String> for BrowserError {
    fn from(value: String) -> Self {
        Self::Native(value)
    }
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display_message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcode_presentation::LanguagePreference;

    #[test]
    fn startup_errors_follow_the_chosen_language_without_matching_english() {
        let en = Localizer::new(LanguagePreference::EnUs);
        let zh = Localizer::new(LanguagePreference::ZhCn);
        for key in [
            TextKey::BrowserCacheUnavailable,
            TextKey::BrowserStartupInvalid,
            TextKey::BrowserStartupStopped,
            TextKey::BrowserProfileInvalid,
            TextKey::BrowserProfileUnwritable,
            TextKey::BrowserProfileLocked,
            TextKey::BrowserWindowRequired,
            TextKey::BrowserMainThreadRequired,
            TextKey::BrowserParentMissing,
        ] {
            let error = BrowserError::Message(key);
            assert_ne!(error.localized(&en), error.localized(&zh));
            assert!(!error.localized(&zh).starts_with("browser-"));
        }
        let error = BrowserError::WithReason(TextKey::BrowserStartupFailed, "EIO".into());
        assert!(error.localized(&zh).contains("EIO"));
        assert!(!error.localized(&zh).contains("Browser"));
    }
}
