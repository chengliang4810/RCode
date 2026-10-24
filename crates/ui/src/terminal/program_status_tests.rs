use super::*;

fn report(statuses: &mut ProgramStatuses, body: &str) {
    statuses.report(body.as_bytes());
}

fn run(decoder: &mut StatusDecoder, bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut forwarded, mut replies) = (Vec::new(), Vec::new());
    decoder.advance(bytes, |event| match event {
        StatusOutput::Bytes(bytes) => forwarded.extend_from_slice(bytes),
        StatusOutput::Reply(bytes) => replies.extend_from_slice(bytes),
    });
    (forwarded, replies)
}

#[test]
fn reports_replace_records_and_last_duplicate_wins() {
    let mut statuses = ProgramStatuses::default();
    report(
        &mut statuses,
        "state=blocked:kind=permission:app=claude-code:msg=QXBwcm92ZT8=",
    );
    assert_eq!(
        statuses.primary().unwrap().1.message.as_deref(),
        Some("Approve?")
    );
    report(
        &mut statuses,
        "state=idle:future=value:bad-pair:UPPER=no:state=working:progress=42",
    );
    let record = statuses.primary().unwrap().1;
    assert_eq!(record.state, ProgramState::Working);
    assert_eq!(record.progress, Some(42));
    assert_eq!(record.kind, None);
    assert_eq!(record.app, None);
    assert_eq!(record.message, None);
}

#[test]
fn child_inherits_nearest_app_and_blocked_child_wins() {
    let mut statuses = ProgramStatuses::default();
    report(&mut statuses, "state=working:app=agent");
    report(&mut statuses, "state=idle:id=workers:app=worker-agent");
    report(&mut statuses, "state=blocked:id=workers/test:kind=question");
    report(&mut statuses, "state=working:id=build");
    assert_eq!(statuses.app("workers/test"), Some("worker-agent"));
    assert_eq!(statuses.app("build"), Some("agent"));
    assert_eq!(statuses.primary().unwrap().0, "workers/test");
    report(&mut statuses, "state=working:id=workers");
    assert_eq!(statuses.app("workers/test"), Some("agent"));
}

#[test]
fn clear_only_removes_addressed_subtree() {
    let mut statuses = ProgramStatuses::default();
    for id in ["a", "a/test", "ab", "a-test"] {
        report(&mut statuses, &format!("state=working:id={id}"));
    }
    report(&mut statuses, "state=clear:id=a");
    assert_eq!(
        statuses.records().map(|(id, _)| id).collect::<Vec<_>>(),
        ["ab", "a-test"]
    );
    report(&mut statuses, "state=clear");
    assert!(statuses.primary().is_none());
}

#[test]
fn malformed_reports_are_atomic_and_invalid_ids_never_replace_root() {
    let mut statuses = ProgramStatuses::default();
    report(&mut statuses, "state=working:app=agent");
    for invalid in [
        "state=done:id=",
        "state=done:id=a//b",
        "state=done:id=/a",
        "state=done:id=a/",
        "state=done:id=bad$name",
        "state=done:id=bad name",
        "state=done:id=中文",
        "state=unknown",
        "app=agent",
        "state=done:msg=ab=c",
        "state=done:msg=Cg==",
        "state=done:title=wog=",            // newline and C1 control
        "state=done:msg=/w==",              // invalid UTF-8
        "state=done:msg=ab=c:msg=dmFsaWQ=", // invalid earlier duplicate
        "state=done:abcdefghijklmnopq=value",
    ] {
        report(&mut statuses, invalid);
        assert_eq!(
            statuses.primary().unwrap().1.state,
            ProgramState::Working,
            "{invalid}"
        );
        assert_eq!(statuses.records().count(), 1);
    }
}

#[test]
fn optional_fields_and_control_free_unicode() {
    let mut statuses = ProgramStatuses::default();
    let encoded =
        base64::engine::general_purpose::STANDARD_NO_PAD.encode("等待批准\u{202e}abc\u{200b}");
    report(
        &mut statuses,
        &format!(" state = blocked :kind=auth:progress=100:msg={encoded}"),
    );
    let record = statuses.primary().unwrap().1;
    assert_eq!(record.kind, Some(BlockedKind::Auth));
    assert_eq!(record.message.as_deref(), Some("等待批准abc"));
    assert_eq!(record.progress, Some(100));
    for progress in ["-1", "+1", "101", "999999999", "1.5", ""] {
        report(
            &mut statuses,
            &format!("state=working:progress={progress}:app=bad/name:kind=permission"),
        );
        let record = statuses.primary().unwrap().1;
        assert_eq!(record.progress, None);
        assert_eq!(record.app, None);
        assert_eq!(record.kind, None);
    }
    report(&mut statuses, "state=done:progress=50:kind=auth");
    assert_eq!(statuses.primary().unwrap().1.progress, None);
}

#[test]
fn all_text_and_identifier_limits_are_checked_before_mutation() {
    let mut statuses = ProgramStatuses::default();
    report(&mut statuses, "state=working");
    for pair in [
        format!("app={}", "a".repeat(33)),
        format!(
            "title={}",
            base64::engine::general_purpose::STANDARD.encode("a".repeat(193))
        ),
        format!(
            "msg={}",
            base64::engine::general_purpose::STANDARD.encode("a".repeat(2049))
        ),
        format!("id={}", "a".repeat(33)),
        format!("id={}", ["a"; 9].join("/")),
        format!("id={}", vec!["a".repeat(32); 4].join("/")),
    ] {
        report(&mut statuses, &format!("state=done:{pair}"));
        assert_eq!(
            statuses.primary().unwrap().1.state,
            ProgramState::Working,
            "{pair}"
        );
    }
    let title = base64::engine::general_purpose::STANDARD.encode("a".repeat(192));
    let msg = base64::engine::general_purpose::STANDARD.encode("a".repeat(2048));
    report(
        &mut statuses,
        &format!("state=done:title={title}:msg={msg}"),
    );
    assert_eq!(
        statuses
            .primary()
            .unwrap()
            .1
            .message
            .as_ref()
            .unwrap()
            .len(),
        2048
    );
}

#[test]
fn record_cap_evicts_least_recently_updated() {
    let mut statuses = ProgramStatuses::default();
    for ix in 0..MAX_RECORDS {
        report(&mut statuses, &format!("state=working:id=t{ix}"));
    }
    report(&mut statuses, "state=done:id=t0");
    report(&mut statuses, "state=working:id=new");
    assert_eq!(statuses.records().count(), MAX_RECORDS);
    assert!(statuses.records.contains_key("t0"));
    assert!(!statuses.records.contains_key("t1"));
}

#[test]
fn stream_handles_every_split_and_both_terminators_without_printing_or_bells() {
    for end in ["\x07", "\x1b\\"] {
        let stream = format!(
            "before\x1b]7501;state=blocked:kind=permission:app=claude-code:msg=QXBwcm92ZT8={end}after"
        );
        for split in 0..=stream.len() {
            let mut decoder = StatusDecoder::default();
            let (mut out, reply) = run(&mut decoder, &stream.as_bytes()[..split]);
            let (tail, tail_reply) = run(&mut decoder, &stream.as_bytes()[split..]);
            out.extend(tail);
            assert_eq!(out, format!("before\x1b]7501;{end}after").as_bytes());
            assert!(reply.is_empty() && tail_reply.is_empty());
            assert_eq!(
                decoder.statuses.primary().unwrap().1.state,
                ProgramState::Blocked
            );
        }
    }
}

#[test]
fn capability_query_replies_without_creating_or_echoing_records() {
    let mut decoder = StatusDecoder::default();
    let mut forwarded = Vec::new();
    for byte in b"\x1b]7501;?\x1b\\" {
        let (out, reply) = run(&mut decoder, &[*byte]);
        forwarded.extend(out);
        if *byte == b'\\' {
            assert_eq!(reply, FEATURES)
        } else {
            assert!(reply.is_empty())
        }
    }
    assert_eq!(forwarded, b"\x1b]7501;\x1b\\");
    assert!(decoder.statuses.primary().is_none());
    assert_eq!(run(&mut decoder, b"\x1b]7501;?\x07").1, FEATURES);
}

#[test]
fn ordinary_terminal_controls_and_unknown_oscs_are_preserved() {
    let data = b"text\x1b[31mred\x1b[0m\x1b]0;title\x07\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\\x1b[?1049h\x1b[?1049l\x1b[6n\x1b]7502;other\x07\x1bPqdata\x1b\\\x1b_soft\x1b\\";
    for split in 0..=data.len() {
        let mut decoder = StatusDecoder::default();
        let (mut out, _) = run(&mut decoder, &data[..split]);
        out.extend(run(&mut decoder, &data[split..]).0);
        assert_eq!(out, data);
        assert!(decoder.statuses.primary().is_none());
    }
}

#[test]
fn controls_inside_escape_sequences_preserve_byte_order() {
    let data = b"\x1b\x07[31mred\x1b\x00]unknown\x07\x1b\x00\x07(0";
    for split in 0..=data.len() {
        let mut decoder = StatusDecoder::default();
        let (mut out, _) = run(&mut decoder, &data[..split]);
        out.extend(run(&mut decoder, &data[split..]).0);
        assert_eq!(out, data);
        assert!(decoder.statuses.primary().is_none());
    }
}

#[test]
fn prompt_exit_and_acknowledgement_have_distinct_lifetimes() {
    let mut decoder = StatusDecoder::default();
    for (id, state) in [
        ("a", "working"),
        ("b", "blocked"),
        ("c", "idle"),
        ("d", "done"),
        ("e", "error"),
    ] {
        run(
            &mut decoder,
            format!("\x1b]7501;state={state}:id={id}\x07").as_bytes(),
        );
    }
    run(&mut decoder, b"\x1b[?1049h\x1b[?1049l\x1b[!p");
    assert_eq!(decoder.statuses.records().count(), 5);
    run(&mut decoder, b"\x1b]133;AB\x07"); // not a prompt marker
    assert_eq!(decoder.statuses.records().count(), 5);
    run(&mut decoder, b"\x1b]133;A;extra\x1b\\");
    assert_eq!(
        decoder
            .statuses
            .records()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        ["d", "e"]
    );
    report(&mut decoder.statuses, "state=working:id=live");
    decoder.statuses.acknowledge();
    assert_eq!(decoder.statuses.primary().unwrap().0, "live");
    report(&mut decoder.statuses, "state=done:id=result");
    decoder.process_exited();
    assert_eq!(decoder.statuses.primary().unwrap().0, "result");
    run(&mut decoder, b"\x1bc");
    assert!(decoder.statuses.primary().is_none());
}

#[test]
fn oversized_or_unterminated_reports_stay_bounded_and_recover() {
    let mut decoder = StatusDecoder::default();
    run(&mut decoder, b"\x1b]7501;state=working");
    for _ in 0..100 {
        run(&mut decoder, &[b'a'; 8192]);
    }
    assert!(decoder.body.len() <= MAX_SEQUENCE);
    assert!(decoder.statuses.primary().is_none());
    let (out, _) = run(&mut decoder, b"\x07visible\x1b]7501;state=done\x07");
    assert_eq!(out, b"\x07visible\x1b]7501;\x07");
    assert_eq!(
        decoder.statuses.primary().unwrap().1.state,
        ProgramState::Done
    );
}

#[test]
fn sequence_limit_counts_prefix_and_terminator() {
    for end in ["\x07", "\x1b\\"] {
        for extra in [0, 1] {
            let mut decoder = StatusDecoder::default();
            let prefix = "\x1b]7501;state=done:x=";
            let padding = MAX_SEQUENCE - prefix.len() - end.len() + extra;
            let frame = format!("{prefix}{}{end}", "a".repeat(padding));
            assert_eq!(
                run(&mut decoder, frame.as_bytes()).0,
                format!("\x1b]7501;{end}").as_bytes()
            );
            assert_eq!(decoder.statuses.primary().is_some(), extra == 0);
        }
    }
}

#[test]
fn cancelled_reports_do_not_leak_and_exit_discards_partial_frame() {
    for cancel in ["\x18", "\x1a", "\x1b[31m"] {
        let mut decoder = StatusDecoder::default();
        run(
            &mut decoder,
            format!("\x1b]7501;state=done{cancel}text").as_bytes(),
        );
        assert!(decoder.statuses.primary().is_none());
        run(&mut decoder, b"\x1b]7501;state=working\x07");
        assert_eq!(
            decoder.statuses.primary().unwrap().1.state,
            ProgramState::Working
        );
    }
    let mut decoder = StatusDecoder::default();
    run(&mut decoder, b"\x1b]7501;state=done");
    decoder.process_exited();
    assert_eq!(run(&mut decoder, b"exited").0, b"exited");
    assert!(decoder.statuses.primary().is_none());
}
