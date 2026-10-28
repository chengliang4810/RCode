//! Statistics derived from durable model-call parts. Context occupancy is never billing.

use std::collections::{HashMap, HashSet};

use crate::{MessagePart, MessageRole, SessionMessageEntry};
use rcode_proto::ModelCallStatistics;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageStatistics {
    pub turns: u64,
    pub calls: u64,
    pub token_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cache reads summed over the calls that reported them.
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// Calls whose response carried cache accounting — the only ones the hit
    /// rate can speak for.
    pub cache_read_calls: u64,
    /// Input tokens of those calls: the hit rate's denominator.
    pub cache_read_input_tokens: u64,
    pub duration_ms: u64,
    pub first_token_ms: u64,
    pub timed_calls: u64,
    pub decode_ms: u64,
    pub decode_tokens: u64,
    pub models: Vec<String>,
    /// Older completed turns have no persisted model-call measurements.
    pub partial_history: bool,
}

impl UsageStatistics {
    fn add(&mut self, call: &ModelCallStatistics) {
        self.duration_ms = self.duration_ms.saturating_add(call.duration_ms);
        if let Some(first) = call
            .first_token_ms
            .filter(|first| *first <= call.duration_ms)
        {
            self.first_token_ms = self.first_token_ms.saturating_add(first);
            self.timed_calls += 1;
            if let Some(output) = call.output_tokens.filter(|_| call.duration_ms > first) {
                self.decode_ms = self.decode_ms.saturating_add(call.duration_ms - first);
                self.decode_tokens = self.decode_tokens.saturating_add(output);
            }
        }
        if let (Some(input), Some(output)) = (call.input_tokens, call.output_tokens) {
            self.token_calls += 1;
            self.input_tokens = self.input_tokens.saturating_add(input);
            self.output_tokens = self.output_tokens.saturating_add(output);
        }
        // Missing cache accounting stays unknown for that call; never turn it
        // into a reported 0%. The sum therefore covers the calls that did
        // report it, and `cache_hit_percent` keeps the denominator to the same
        // calls — one silent response no longer blanks the whole session.
        self.cache_read_tokens = match (self.cache_read_tokens, call.cache_read_tokens) {
            (Some(total), Some(read)) => Some(total.saturating_add(read)),
            (previous, next) => previous.or(next),
        };
        if let Some(input) = call
            .input_tokens
            .filter(|_| call.cache_read_tokens.is_some())
        {
            self.cache_read_calls += 1;
            self.cache_read_input_tokens = self.cache_read_input_tokens.saturating_add(input);
        }
        self.cache_write_tokens = match (self.cache_write_tokens, call.cache_write_tokens) {
            (Some(a), Some(b)) => Some(a.saturating_add(b)),
            (previous, next) => previous.or(next),
        };
        self.calls += 1;
        if !call.model.is_empty() && !self.models.contains(&call.model) {
            self.models.push(call.model.clone());
        }
    }

    pub fn total_tokens(&self) -> Option<u64> {
        (self.calls > 0 && self.token_calls == self.calls)
            .then(|| self.input_tokens.saturating_add(self.output_tokens))
    }

    /// Cache reads as a share of the input that reported them.
    ///
    /// Only calls that carried cache accounting count. A provider or a gateway
    /// route may omit the counters for some responses; folding an unknown into
    /// the sum would either hide the rate for the whole session or invent one,
    /// so the rate spans the reporting calls and [`Self::cache_coverage_complete`]
    /// says whether that is all of them.
    pub fn cache_hit_percent(&self) -> Option<f64> {
        self.total_tokens()?;
        let cached = self.cache_read_tokens?;
        (self.cache_read_input_tokens > 0 && cached <= self.cache_read_input_tokens)
            .then(|| cached as f64 * 100.0 / self.cache_read_input_tokens as f64)
    }

    /// Whether every recorded call reported cache reads, so the rate and the
    /// uncached input describe the whole session rather than a subset.
    pub fn cache_coverage_complete(&self) -> bool {
        self.cache_read_calls >= self.calls
    }

    pub fn tokens_per_second(&self) -> Option<f64> {
        (self.decode_ms > 0).then(|| self.decode_tokens as f64 * 1000.0 / self.decode_ms as f64)
    }
}

#[derive(Default)]
pub struct ConversationStatistics {
    pub session: UsageStatistics,
    /// One control on the last assistant entry of each user turn.
    pub turns: HashMap<String, UsageStatistics>,
}

impl ConversationStatistics {
    pub fn from_entries(entries: &[SessionMessageEntry]) -> Self {
        let mut result = Self::default();
        let mut turn = UsageStatistics::default();
        let mut last_entry = None;
        let mut completed_content = false;
        let mut seen = HashSet::new();
        for entry in entries {
            if entry.role == MessageRole::User {
                if completed_content && turn.calls == 0 {
                    result.session.partial_history = true;
                }
                completed_content = false;
                if let Some(id) = last_entry.take() {
                    if turn.calls > 0 {
                        turn.turns = 1;
                        result.session.turns += 1;
                        result.turns.insert(id, std::mem::take(&mut turn));
                    }
                }
            } else if entry.role == MessageRole::Assistant {
                completed_content |= entry.status == Some(crate::MessageStatus::Complete)
                    && entry.parts.iter().any(
                        |part| matches!(part, MessagePart::Text { text, .. } if !text.is_empty()),
                    );
                last_entry = Some(entry.continuation_of.as_ref().unwrap_or(&entry.id).clone());
                for part in &entry.parts {
                    if let MessagePart::Usage { id, statistics } = part {
                        if seen.insert(id) {
                            result.session.add(statistics);
                            turn.add(statistics);
                        }
                    }
                }
            }
        }
        if let Some(id) = last_entry {
            if completed_content && turn.calls == 0 {
                result.session.partial_history = true;
            }
            if turn.calls > 0 {
                turn.turns = 1;
                result.session.turns += 1;
                result.turns.insert(id, turn);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MessageStatus, SegmentWriter, SessionDoc, fold_event_into_parts};

    fn call(
        id: &str,
        input: u64,
        output: u64,
        read: Option<u64>,
        duration: u64,
        first: Option<u64>,
    ) -> MessagePart {
        MessagePart::Usage {
            id: id.into(),
            statistics: ModelCallStatistics {
                model: "provider/model".into(),
                input_tokens: Some(input),
                output_tokens: Some(output),
                cache_read_tokens: read,
                cache_write_tokens: Some(0),
                duration_ms: duration,
                first_token_ms: first,
            },
        }
    }

    fn entry(id: &str, role: MessageRole, parts: Vec<MessagePart>) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role,
            parts,
            created_at: 0,
            device_id: "test".into(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
            excluded: false,
        }
    }

    #[test]
    fn statistics_aggregate_calls_per_user_turn_and_weight_decode_speed() {
        let mut second = entry(
            "a1-cont",
            MessageRole::Assistant,
            vec![call("c2", 300, 30, Some(100), 2000, Some(500))],
        );
        second.continuation_of = Some("a1".into());
        let entries = vec![
            entry("u1", MessageRole::User, vec![]),
            entry(
                "a1",
                MessageRole::Assistant,
                vec![call("c1", 100, 10, Some(100), 1000, Some(500))],
            ),
            second,
            entry("u2", MessageRole::User, vec![]),
            entry(
                "a2",
                MessageRole::Assistant,
                vec![call("c3", 600, 60, Some(400), 4000, Some(1000))],
            ),
        ];
        let stats = ConversationStatistics::from_entries(&entries);
        assert_eq!((stats.session.turns, stats.session.calls), (2, 3));
        assert_eq!(stats.session.total_tokens(), Some(1100));
        assert_eq!(stats.session.cache_hit_percent(), Some(60.0));
        assert_eq!(stats.session.tokens_per_second(), Some(20.0));
        assert_eq!(
            (stats.session.first_token_ms, stats.session.timed_calls),
            (2000, 3)
        );
        assert_eq!(stats.turns["a1"].total_tokens(), Some(440));
        assert_eq!(stats.turns["a2"].total_tokens(), Some(660));
    }

    #[test]
    fn statistics_unknown_counters_and_zero_are_distinct() {
        let stats = ConversationStatistics::from_entries(&[entry(
            "a",
            MessageRole::Assistant,
            vec![call("c1", 0, 0, Some(0), 0, None)],
        )]);
        assert_eq!(stats.session.total_tokens(), Some(0));
        assert_eq!(stats.session.cache_hit_percent(), None);
        assert_eq!(stats.session.tokens_per_second(), None);
        let stats = ConversationStatistics::from_entries(&[entry(
            "a",
            MessageRole::Assistant,
            vec![
                call("c1", 100, 20, Some(50), 100, None),
                call("c2", 200, 40, None, 200, None),
            ],
        )]);
        assert_eq!(stats.session.total_tokens(), Some(360));
        // The unknown call stays out of the rate instead of blanking it.
        assert_eq!(stats.session.cache_hit_percent(), Some(50.0));
        assert_eq!(
            (
                stats.session.cache_read_calls,
                stats.session.cache_read_input_tokens,
            ),
            (1, 100)
        );
        assert!(!stats.session.cache_coverage_complete());
        let stats = ConversationStatistics::from_entries(&[entry(
            "a",
            MessageRole::Assistant,
            vec![MessagePart::Usage {
                id: "unknown".into(),
                statistics: ModelCallStatistics::default(),
            }],
        )]);
        assert_eq!(stats.session.total_tokens(), None);
        assert_eq!(stats.session.cache_hit_percent(), None);
        assert!(!stats.session.cache_coverage_complete());
    }

    #[test]
    fn statistics_old_history_is_explicit_and_mixed_cache_writes_are_preserved() {
        let mut write = call("write", 1000, 100, Some(500), 1000, None);
        if let MessagePart::Usage { statistics, .. } = &mut write {
            statistics.cache_write_tokens = Some(200);
        }
        let mut read = call("read", 500, 100, Some(400), 1000, None);
        if let MessagePart::Usage { statistics, .. } = &mut read {
            statistics.cache_write_tokens = None;
        }
        let stats = ConversationStatistics::from_entries(&[
            entry("old-user", MessageRole::User, vec![]),
            entry(
                "old-assistant",
                MessageRole::Assistant,
                vec![MessagePart::Text {
                    id: "old".into(),
                    text: "Historical reply".into(),
                }],
            ),
            entry("new-user", MessageRole::User, vec![]),
            entry("new-assistant", MessageRole::Assistant, vec![write, read]),
        ]);
        assert!(stats.session.partial_history);
        assert_eq!(stats.session.cache_write_tokens, Some(200));
        assert_eq!(stats.session.cache_hit_percent(), Some(60.0));
        assert_eq!(stats.session.total_tokens(), Some(1700));
    }

    /// A gateway may answer some requests without cache counters. Those calls
    /// stay out of the rate instead of blanking it, and the coverage fields say
    /// how much of the session the number speaks for.
    #[test]
    fn statistics_cache_rate_covers_only_the_calls_that_reported_it() {
        let mut second = entry(
            "a1-cont",
            MessageRole::Assistant,
            vec![call("c2", 2000, 100, None, 2000, Some(500))],
        );
        second.continuation_of = Some("a1".into());
        let entries = vec![
            entry("u1", MessageRole::User, vec![]),
            entry(
                "a1",
                MessageRole::Assistant,
                vec![call("c1", 1000, 100, Some(900), 1000, Some(200))],
            ),
            second,
            entry("u2", MessageRole::User, vec![]),
            entry(
                "a2",
                MessageRole::Assistant,
                vec![call("c3", 4000, 100, Some(2000), 4000, Some(800))],
            ),
        ];
        let stats = ConversationStatistics::from_entries(&entries);
        assert_eq!(stats.session.calls, 3);
        assert_eq!(stats.session.cache_read_calls, 2);
        assert_eq!(stats.session.cache_read_input_tokens, 5000);
        assert_eq!(stats.session.cache_read_tokens, Some(2900));
        // 2900 over the 5000 input tokens that reported it, not over all 7300.
        assert_eq!(stats.session.cache_hit_percent(), Some(58.0));
        assert!(!stats.session.cache_coverage_complete());
        let first = &stats.turns["a1"];
        assert_eq!((first.calls, first.cache_read_calls), (2, 1));
        assert_eq!(first.cache_hit_percent(), Some(90.0));
        assert!(!first.cache_coverage_complete());
        let second = &stats.turns["a2"];
        assert_eq!(second.cache_hit_percent(), Some(50.0));
        assert!(second.cache_coverage_complete());
        // Every call reporting keeps the plain whole-session reading.
        let reported = ConversationStatistics::from_entries(&[entry(
            "a",
            MessageRole::Assistant,
            vec![
                call("c1", 1000, 100, Some(900), 1000, Some(200)),
                call("c2", 1000, 100, Some(100), 1000, Some(200)),
            ],
        )]);
        assert_eq!(reported.session.cache_hit_percent(), Some(50.0));
        assert!(reported.session.cache_coverage_complete());
    }

    #[test]
    fn statistics_round_trip_rebuild_and_compaction_keep_accounting() {
        let doc = SessionDoc::init("stats").unwrap();
        let part = call("call-1", 1000, 200, Some(900), 4000, Some(1000));
        let mut message = entry("a", MessageRole::Assistant, vec![part]);
        message.excluded = true;
        doc.push_message(&message).unwrap();
        let restored = SessionDoc::from_doc(loro::LoroDoc::new());
        restored
            .doc()
            .import(&doc.export_snapshot().unwrap())
            .unwrap();
        assert_eq!(restored.read_entries().unwrap(), vec![message]);
        let thin = crate::rebuild::rebuild_thin_doc(&restored).unwrap();
        assert_eq!(
            ConversationStatistics::from_entries(&thin.doc.read_entries().unwrap())
                .session
                .total_tokens(),
            Some(1200)
        );
    }

    #[test]
    fn statistics_duplicate_events_update_one_durable_call() {
        let MessagePart::Usage { id, statistics } =
            call("call-1", 100, 10, Some(80), 1000, Some(100))
        else {
            unreachable!()
        };
        let event = rcode_proto::AgentEvent::ModelCall { id, statistics };
        let mut parts = Vec::new();
        fold_event_into_parts(&mut parts, &event);
        fold_event_into_parts(&mut parts, &event);
        assert_eq!(parts.len(), 1);
        let doc = SessionDoc::init("streaming-stats").unwrap();
        let mut segment = SegmentWriter::begin(&doc, "assistant", "device", 0).unwrap();
        segment.sync(&parts).unwrap();
        segment.sync(&parts).unwrap();
        assert_eq!(
            ConversationStatistics::from_entries(&doc.read_entries().unwrap())
                .session
                .total_tokens(),
            Some(110)
        );
    }
}
