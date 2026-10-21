#[cfg(test)]
mod tests {
    use crate::{LanguagePreference, Locale, Localizer, TextError, TextKey, resolve};

    fn zh() -> Localizer {
        Localizer::with_system_locale(LanguagePreference::ZhCn, None)
    }

    fn en() -> Localizer {
        Localizer::with_system_locale(LanguagePreference::EnUs, None)
    }

    // ─── Resolution ─────────────────────────────────────────────────────────

    #[test]
    fn the_default_preference_follows_the_system() {
        assert_eq!(LanguagePreference::default(), LanguagePreference::System);
    }

    #[test]
    fn a_system_locale_whose_primary_subtag_is_chinese_resolves_to_chinese() {
        // Every Chinese tag resolves the same way: the application ships one
        // Chinese translation, and offering none of them would be worse than
        // offering simplified script to a traditional-script reader.
        for locale in ["zh", "zh-CN", "zh-Hans", "zh-Hant", "zh-TW", "zh_CN"] {
            assert_eq!(
                resolve(LanguagePreference::System, Some(locale)),
                Locale::ZhCn,
                "`{locale}` should resolve to Chinese"
            );
        }
    }

    #[test]
    fn every_other_system_locale_resolves_to_english() {
        for locale in ["en", "en-US", "fr", "ja-JP", "de-AT", "ko"] {
            assert_eq!(
                resolve(LanguagePreference::System, Some(locale)),
                Locale::EnUs,
                "`{locale}` should resolve to English"
            );
        }
    }

    #[test]
    fn a_system_locale_that_cannot_be_read_resolves_to_english() {
        // Not a panic, not Chinese, not an empty string: a locale the library
        // could not read is not a reason to show nothing.
        assert_eq!(resolve(LanguagePreference::System, None), Locale::EnUs);
        assert_eq!(resolve(LanguagePreference::System, Some("")), Locale::EnUs);
    }

    #[test]
    fn a_manual_choice_overrides_the_system_whichever_way_it_points() {
        // Someone who sets English on a Chinese machine meant it.
        assert_eq!(
            resolve(LanguagePreference::EnUs, Some("zh-CN")),
            Locale::EnUs
        );
        assert_eq!(
            resolve(LanguagePreference::ZhCn, Some("en-US")),
            Locale::ZhCn
        );
    }

    #[test]
    fn an_unknown_stored_preference_falls_back_to_system_rather_than_failing() {
        // A settings file written by a newer build, or edited by hand, should
        // cost the language choice and nothing else — refusing to parse the
        // whole file would take the memory and skill switches with it.
        for value in ["", "System", "zh-cn", "klingon", "{}"] {
            assert_eq!(
                LanguagePreference::parse(value),
                LanguagePreference::System,
                "`{value}` should fall back to System"
            );
        }
    }

    #[test]
    fn a_preference_round_trips_through_its_stored_name() {
        for preference in LanguagePreference::ALL {
            assert_eq!(LanguagePreference::parse(preference.as_str()), preference);
        }
    }

    // ─── Looking up ─────────────────────────────────────────────────────────

    #[test]
    fn a_message_comes_back_in_the_resolved_language() {
        assert_eq!(en().text(TextKey::ShellNewSession), "New session");
        assert_eq!(zh().text(TextKey::ShellNewSession), "新建会话");
    }

    #[test]
    fn a_message_with_arguments_is_formatted() {
        assert_eq!(
            en().format_with(TextKey::ErrorModelUnknown, &[("model", "gpt-5")]),
            "No model is configured with the id gpt-5."
        );
        assert_eq!(
            zh().format_with(TextKey::ErrorModelUnknown, &[("model", "gpt-5")]),
            "没有配置 id 为 gpt-5 的模型。"
        );
    }

    #[test]
    fn english_plural_rules_select_the_right_form() {
        // Fluent handles this, which is the point of using it rather than
        // string concatenation: English has a distinct singular.
        assert_eq!(
            en().format_count(TextKey::CountSessions, "count", 1),
            "1 session"
        );
        assert_eq!(
            en().format_count(TextKey::CountSessions, "count", 2),
            "2 sessions"
        );
        assert_eq!(
            en().format_count(TextKey::CountSessions, "count", 0),
            "0 sessions"
        );
    }

    #[test]
    fn chinese_has_no_separate_singular() {
        assert_eq!(
            zh().format_count(TextKey::CountSessions, "count", 1),
            "1 个会话"
        );
        assert_eq!(
            zh().format_count(TextKey::CountSessions, "count", 5),
            "5 个会话"
        );
    }

    #[test]
    fn every_generated_key_resolves_in_both_languages() {
        // The build checks the key sets match; this checks each one actually
        // formats. A message that parses but fails to format would otherwise
        // only show up when the user reached that screen.
        //
        // Every message is retried with a plausible argument per name it uses,
        // because the argument-less lookup cannot format one that needs a
        // number — and "could not format without args" is not the failure this
        // test is looking for.
        for key in TextKey::ALL {
            for localizer in [en(), zh()] {
                // One attempt per argument name the message might use; a
                // message with no placeables formats on the first try.
                // The build emits each key's variable names, so every one the
                // message needs is supplied — a message taking two values
                // cannot be satisfied one name at a time.
                let args: Vec<(&str, &str)> =
                    key.variables().iter().map(|name| (*name, "x")).collect();
                let formatted = localizer.format_with(*key, &args);
                // A failed lookup returns the key itself, so that is the
                // signal — an empty message would be a formatting bug.
                let resolved = !formatted.is_empty() && formatted != key.id();
                assert!(
                    resolved,
                    "`{}` did not resolve in {}",
                    key.id(),
                    localizer.locale().tag()
                );
            }
        }
    }

    #[test]
    fn strict_lookup_succeeds_for_a_key_the_resources_define() {
        assert_eq!(
            en().try_text(TextKey::ShellArchive).expect("resolves"),
            "Archive"
        );
    }

    #[test]
    fn the_error_does_not_carry_the_arguments() {
        // The error goes to a log, and an argument may be a path, a provider's
        // message or the user's own text.
        let error = TextError::Missing {
            key: "some-key".to_owned(),
        };
        assert_eq!(error.to_string(), "the resource for `some-key` is missing");
    }

    #[test]
    fn a_localizer_is_cheap_to_clone_and_keeps_its_language() {
        // Windows hold their own; a clone must not re-resolve against a system
        // locale that may differ by the time it is read.
        let zh = zh();
        let clone = zh.clone();
        assert_eq!(clone.locale(), Locale::ZhCn);
        assert_eq!(
            clone.text(TextKey::ShellNewSession),
            zh.text(TextKey::ShellNewSession)
        );
    }

    #[test]
    fn every_declared_locale_has_a_language_tag() {
        for locale in [Locale::EnUs, Locale::ZhCn] {
            assert!(!locale.tag().is_empty());
            // The tag must be one Fluent accepts, or the bundle would not build.
            let _ = locale.language_identifier();
        }
    }

    #[test]
    fn the_generated_keys_are_unique() {
        // `ALL` is generated from a `BTreeMap`, so it is sorted — and a
        // duplicate id would mean two variants for one message.
        let ids: Vec<&str> = TextKey::ALL.iter().map(|key| key.id()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "a key id appears twice");
    }

    #[test]
    fn the_key_id_is_the_resource_key() {
        // The variant name is derived from the key; the id it reports must be
        // the key itself, or lookups would miss.
        assert_eq!(TextKey::ShellNewSession.id(), "shell-new-session");
        assert!(
            TextKey::ShellNewSession
                .id()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "keys are lowercase-hyphenated"
        );
    }
}
