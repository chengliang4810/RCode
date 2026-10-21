//! Relative times — "5 minutes ago" — for the rows that stamp *when*.
//!
//! Moved here from `rcode_proto::view` (§2.5 "展示逻辑迁移"). The original
//! returned a compact `5m` with an English-only literal `"now"`; a translated
//! UI needs whole words and a plural rule ("1 minute ago" / "2 minutes ago"),
//! which Fluent supplies and a `format!` cannot. The unit thresholds are
//! unchanged from the original port of the original app's `formatTimeAgo` — only the
//! wording is now a resource.

use chrono::{DateTime, Utc};

use crate::{Localizer, TextKey};

/// How long ago `then` was, in words, in the current language.
///
/// # Future timestamps
///
/// A `then` ahead of `now` is clamped to `now`, so it reads as "just now". A
/// relative time appears on rows the user just created, and a stamp can be a
/// few seconds ahead of this machine's clock — another device wrote it, or the
/// clock stepped back. "in 2 minutes" or a bare "-2 minutes" on a row the user
/// is looking at reads as a bug, not as information, so the past-only clamp is
/// deliberate rather than an accident of `max(0)`.
pub fn format_time_ago(localizer: &Localizer, then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(then).num_seconds().max(0);
    // Under a minute reads as "just now" — otherwise 45–59s would floor to a
    // bare "0 minutes ago".
    if seconds < 60 {
        return localizer.text(TextKey::TimeJustNow);
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return localizer.format_count(TextKey::TimeMinutesAgo, "count", minutes);
    }
    let hours = minutes / 60;
    if hours < 24 {
        return localizer.format_count(TextKey::TimeHoursAgo, "count", hours);
    }
    let days = hours / 24;
    if days < 7 {
        return localizer.format_count(TextKey::TimeDaysAgo, "count", days);
    }
    let weeks = days / 7;
    if weeks < 5 {
        return localizer.format_count(TextKey::TimeWeeksAgo, "count", weeks);
    }
    let months = days / 30;
    if months < 12 {
        return localizer.format_count(TextKey::TimeMonthsAgo, "count", months);
    }
    localizer.format_count(TextKey::TimeYearsAgo, "count", days / 365)
}

/// Compact relative time for dense rows, localized without an English-only unit suffix.
pub fn format_time_compact(
    localizer: &Localizer,
    then: DateTime<Utc>,
    now: DateTime<Utc>,
) -> String {
    let seconds = now.signed_duration_since(then).num_seconds().max(0);
    if seconds < 60 {
        return localizer.text(TextKey::CompactTimeJustNow);
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return localizer.format_count(TextKey::CompactTimeMinutesAgo, "count", minutes);
    }
    let hours = minutes / 60;
    if hours < 24 {
        return localizer.format_count(TextKey::CompactTimeHoursAgo, "count", hours);
    }
    let days = hours / 24;
    if days < 7 {
        return localizer.format_count(TextKey::CompactTimeDaysAgo, "count", days);
    }
    let weeks = days / 7;
    if weeks < 5 {
        return localizer.format_count(TextKey::CompactTimeWeeksAgo, "count", weeks);
    }
    let months = days / 30;
    if months < 12 {
        return localizer.format_count(TextKey::CompactTimeMonthsAgo, "count", months);
    }
    localizer.format_count(TextKey::CompactTimeYearsAgo, "count", days / 365)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LanguagePreference;
    use chrono::Duration;

    fn en() -> Localizer {
        Localizer::with_system_locale(LanguagePreference::EnUs, None)
    }

    fn zh() -> Localizer {
        Localizer::with_system_locale(LanguagePreference::ZhCn, None)
    }

    /// `then` is this many seconds before `now`.
    fn ago(seconds: i64) -> (DateTime<Utc>, DateTime<Utc>) {
        let now = DateTime::<Utc>::UNIX_EPOCH + Duration::days(10_000);
        (now - Duration::seconds(seconds), now)
    }

    #[test]
    fn under_a_minute_reads_as_just_now() {
        for seconds in [0, 1, 45, 59] {
            let (then, now) = ago(seconds);
            assert_eq!(format_time_ago(&en(), then, now), "just now");
            assert_eq!(format_time_ago(&zh(), then, now), "刚刚");
        }
    }

    #[test]
    fn a_future_timestamp_reads_as_just_now_rather_than_negative() {
        // A row stamped a few seconds ahead by another device's clock, or after
        // this clock stepped back. "-3 minutes" on a row the user just created
        // would read as a bug; clamping to the present is the honest display.
        let now = DateTime::<Utc>::UNIX_EPOCH + Duration::days(10_000);
        let future = now + Duration::minutes(3);
        assert_eq!(format_time_ago(&en(), future, now), "just now");
        assert_eq!(format_time_ago(&zh(), future, now), "刚刚");
    }

    #[test]
    fn each_threshold_switches_unit_at_its_boundary() {
        // The values either side of every boundary, so an off-by-one in the
        // comparison cannot pass unnoticed.
        let cases = [
            (60, "1 minute ago"),
            (59 * 60, "59 minutes ago"),
            (60 * 60, "1 hour ago"),
            (23 * 3600 + 3599, "23 hours ago"),
            (24 * 3600, "1 day ago"),
            (6 * 86400, "6 days ago"),
            (7 * 86400, "1 week ago"),
            // 4 weeks is still weeks; the month branch only starts at 5 weeks
            // (days/7 < 5), so 28 days floors to 4 weeks rather than a month.
            (4 * 7 * 86400, "4 weeks ago"),
            (5 * 7 * 86400, "1 month ago"),
            (11 * 30 * 86400, "11 months ago"),
            (400 * 86400, "1 year ago"),
        ];
        for (seconds, expected) in cases {
            let (then, now) = ago(seconds);
            assert_eq!(
                format_time_ago(&en(), then, now),
                expected,
                "after {seconds}s"
            );
        }
    }

    #[test]
    fn compact_rows_localize_day_units() {
        let (then, now) = ago(6 * 86400);
        assert_eq!(format_time_compact(&en(), then, now), "6d");
        assert_eq!(format_time_compact(&zh(), then, now), "6天");
    }

    #[test]
    fn compact_rows_use_the_same_thresholds_and_clamp_future_times() {
        for (seconds, english, chinese) in [
            (0, "now", "刚刚"),
            (59, "now", "刚刚"),
            (60, "1m", "1分钟"),
            (3600, "1h", "1小时"),
            (86400, "1d", "1天"),
            (7 * 86400, "1w", "1周"),
            (35 * 86400, "1mo", "1个月"),
            (365 * 86400, "1y", "1年"),
        ] {
            let (then, now) = ago(seconds);
            assert_eq!(format_time_compact(&en(), then, now), english);
            assert_eq!(format_time_compact(&zh(), then, now), chinese);
        }
        let now = DateTime::<Utc>::UNIX_EPOCH + Duration::days(10_000);
        assert_eq!(
            format_time_compact(&zh(), now + Duration::minutes(2), now),
            "刚刚"
        );
    }
    #[test]
    fn english_selects_the_singular_for_one() {
        // The plural rule is the reason Fluent is used here: "1 minute ago" and
        // "2 minutes ago" cannot both come from a `format!("{n} minutes ago")`.
        let (then, now) = ago(60);
        assert_eq!(format_time_ago(&en(), then, now), "1 minute ago");
        let (then, now) = ago(120);
        assert_eq!(format_time_ago(&en(), then, now), "2 minutes ago");
    }

    #[test]
    fn chinese_uses_a_complete_quantity_expression() {
        // Chinese has no separate singular, so one form covers every count.
        for (seconds, expected) in [(60, "1 分钟前"), (120, "2 分钟前"), (3600, "1 小时前")]
        {
            let (then, now) = ago(seconds);
            assert_eq!(format_time_ago(&zh(), then, now), expected);
        }
    }
}
