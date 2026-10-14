#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! A transcript keeps the history an agent compacted away. A transfer must
//! carry only what the source agent still saw, or the target starts with a
//! prompt that is too long to continue or to compact.

use serde_json::json;
use txcript::harness::claude_code::{self, Record};
use txcript::harness::codex::{self, Line};
use txcript::harness::opencode::{self, Export, MessageRecord};

fn codex_line(kind: &str, payload: serde_json::Value) -> Line {
    Line {
        timestamp: Some("2026-01-01T00:00:00.000Z".into()),
        kind: kind.into(),
        payload,
        extra: serde_json::Map::new(),
    }
}

fn codex_message(role: &str, text: &str) -> Line {
    codex_line(
        "response_item",
        json!({"type": "message", "role": role, "content": [{"type": "input_text", "text": text}]}),
    )
}

#[test]
fn codex_resumes_from_the_last_compaction() {
    let lines = vec![
        codex_line("session_meta", json!({"id": "s"})),
        codex_message("user", "old question"),
        codex_line(
            "compacted",
            json!({"replacement_history": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "kept question"}]}]}),
        ),
        codex_message("user", "newer"),
        codex_line(
            "compacted",
            json!({"replacement_history": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "kept"}]},
                {"type": "compaction", "encrypted_content": "x"}
            ]}),
        ),
        codex_message("user", "after"),
    ];
    let active = codex::active_context(&lines);
    let kinds: Vec<&str> = active.iter().map(|l| l.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "session_meta",
            "response_item",
            "response_item",
            "response_item"
        ]
    );
    assert_eq!(active[1].payload["content"][0]["text"], "kept");
    assert_eq!(active[3].payload["content"][0]["text"], "after");
}

#[test]
fn codex_without_compaction_is_unchanged() {
    let lines = vec![
        codex_line("session_meta", json!({"id": "s"})),
        codex_message("user", "hi"),
    ];
    assert_eq!(codex::active_context(&lines), lines);
}

#[test]
fn claude_resumes_from_the_last_boundary() {
    let record = |value: serde_json::Value| Record::from(value);
    let boundary = || record(json!({"type": "system", "subtype": "compact_boundary", "uuid": "b"}));
    let other = |n: u32| record(json!({"type": "system", "subtype": "turn_duration", "n": n}));
    let records = vec![
        other(0),
        boundary(),
        other(1),
        boundary(),
        other(2),
        other(3),
    ];
    let active = claude_code::active_context(&records);
    assert_eq!(active.len(), 3);
    assert_eq!(active[0], boundary());
    assert_eq!(active[2], other(3));
    assert_eq!(claude_code::active_context(&[other(0)]).len(), 1);
}

#[test]
fn opencode_keeps_summary_then_tail_then_newer_messages() {
    let message =
        |id: &str, role: &str, extra: serde_json::Value, parts: Vec<serde_json::Value>| {
            let mut info = json!({"id": id, "role": role});
            info.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            MessageRecord { info, parts }
        };
    let export = Export {
        info: json!({}),
        messages: vec![
            message("m1", "user", json!({}), vec![]),
            message("m2", "assistant", json!({}), vec![]),
            message("m3", "user", json!({}), vec![]),
            message("m4", "assistant", json!({}), vec![]),
            message(
                "m5",
                "user",
                json!({}),
                vec![json!({"type": "compaction", "tail_start_id": "m3"})],
            ),
            message(
                "m6",
                "assistant",
                json!({"summary": true, "parentID": "m5"}),
                vec![],
            ),
            message("m7", "user", json!({}), vec![]),
        ],
    };
    let ids: Vec<String> = opencode::active_context(&export)
        .messages
        .iter()
        .map(|m| m.info["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["m6", "m3", "m4", "m7"]);
}
