//! Language-aware units for recorded conversation measurements.
use crate::{Locale, Localizer, TextKey};

pub fn duration(l: &Localizer, ms: u64) -> String {
    if ms < 1_000 {
        return l.format_with(
            TextKey::ConversationStatsMilliseconds,
            &[("value", &ms.to_string())],
        );
    }
    // Round whole seconds before decomposing so minute/hour carries stay valid.
    let mut seconds = ms / 1_000 + u64::from(ms % 1_000 >= 500);
    let mut parts = Vec::new();
    for (unit, key) in [
        (86_400, TextKey::ConversationStatsDays),
        (3_600, TextKey::ConversationStatsHours),
        (60, TextKey::ConversationStatsMinutes),
    ] {
        let value = seconds / unit;
        seconds %= unit;
        if value > 0 {
            parts.push(l.format_with(key, &[("value", &value.to_string())]));
        }
    }
    if seconds > 0 || parts.is_empty() {
        let value = seconds.to_string();
        parts.push(l.format_with(TextKey::ConversationStatsSeconds, &[("seconds", &value)]));
    }
    parts.join(if l.locale() == Locale::ZhCn { "" } else { " " })
}

fn decimal(value: f64) -> String {
    format!("{value:.1}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

pub fn tokens(l: &Localizer, value: u64) -> String {
    let units: &[(f64, TextKey)] = match l.locale() {
        Locale::ZhCn => &[
            (
                100_000_000.0,
                TextKey::ConversationStatsTokensHundredMillion,
            ),
            (10_000.0, TextKey::ConversationStatsTokensTenThousand),
            (1_000.0, TextKey::ConversationStatsTokensThousand),
        ],
        Locale::EnUs => &[
            (1_000_000_000.0, TextKey::ConversationStatsTokensBillion),
            (1_000_000.0, TextKey::ConversationStatsTokensMillion),
            (1_000.0, TextKey::ConversationStatsTokensThousand),
        ],
    };
    // Promote values that round up to the next unit (999,999 -> 1M).
    for (index, &(scale, key)) in units.iter().enumerate() {
        let threshold = units
            .get(index + 1)
            .map_or(scale, |(lower, _)| scale - lower / 20.0);
        if value as f64 >= threshold {
            return l.format_with(key, &[("value", &decimal(value as f64 / scale))]);
        }
    }
    l.format_with(
        TextKey::ConversationStatsTokens,
        &[("value", &value.to_string())],
    )
}

pub fn token_rate(l: &Localizer, value: f64) -> String {
    let tokens = tokens(l, value.round().max(0.0) as u64);
    l.format_with(TextKey::ConversationStatsTokenRate, &[("tokens", &tokens)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LanguagePreference;

    #[test]
    fn duration_converts_units_and_carries_rounding_in_both_languages() {
        let zh = Localizer::new(LanguagePreference::ZhCn);
        let en = Localizer::new(LanguagePreference::EnUs);
        for (ms, chinese, english) in [
            (0, "0毫秒", "0ms"),
            (450, "450毫秒", "450ms"),
            (4_400, "4秒", "4s"),
            (4_500, "5秒", "5s"),
            (59_500, "1分", "1m"),
            (59_999, "1分", "1m"),
            (2_378_500, "39分39秒", "39m 39s"),
            (3_599_500, "1小时", "1h"),
            (3_723_400, "1小时2分3秒", "1h 2m 3s"),
            (86_400_000, "1天", "1d"),
        ] {
            assert_eq!(duration(&zh, ms), chinese);
            assert_eq!(duration(&en, ms), english);
        }
    }

    #[test]
    fn token_units_and_rates_follow_the_display_language() {
        let zh = Localizer::new(LanguagePreference::ZhCn);
        let en = Localizer::new(LanguagePreference::EnUs);
        for (count, chinese, english) in [
            (0, "0 Token", "0 tokens"),
            (999, "999 Token", "999 tokens"),
            (1_000, "1千 Token", "1K tokens"),
            (10_419_396, "1041.9万 Token", "10.4M tokens"),
            (338_989, "33.9万 Token", "339K tokens"),
            (100_000_000, "1亿 Token", "100M tokens"),
            (1_000_000_000, "10亿 Token", "1B tokens"),
        ] {
            assert_eq!(tokens(&zh, count), chinese);
            assert_eq!(tokens(&en, count), english);
        }
        assert_eq!(tokens(&en, 999_999), "1M tokens");
        assert_eq!(token_rate(&zh, 168.2), "168 Token/秒");
        assert_eq!(token_rate(&en, 168.2), "168 tokens/s");
    }
}
