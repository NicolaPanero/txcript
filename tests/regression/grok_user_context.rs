#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use serde_json::{Value, json};
use txcript::common::{Block, Role};
use txcript::harness::{codex::Codex, grok::Grok};
use txcript::{Codec, TextCodec, convert};

#[allow(clippy::needless_pass_by_value)]
fn session(history: Vec<Value>, updates: Vec<Value>) -> txcript::Transcript<Grok> {
    Grok::from_text(&json!({"chat_history":history,"updates":updates}).to_string()).unwrap()
}

fn user_texts(common: &txcript::Transcript<txcript::Common>) -> Vec<String> {
    common
        .body
        .iter()
        .filter(|m| m.role == Role::User)
        .flat_map(|m| &m.content)
        .filter_map(|b| {
            if let Block::Text { text } = b {
                Some(text.clone())
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn runtime_injections_are_not_forged_user_messages_in_codex() {
    let mut history = Vec::new();
    for reason in [
        "session_prefix",
        "project_instructions",
        "system_reminder",
        "auto_continue",
        "working_directory_switch",
        "primary",
        "auto_recovery",
        "length_continue",
    ] {
        history.push(json!({"type":"user","synthetic_reason":reason,"content":[{"type":"text","text":format!("internal {reason}")}]}));
    }
    history.push(json!({"type":"user","synthetic_reason":"compaction_meta","content":[{"type":"text","text":"Summary: implemented parser; next fix tests."}]}));
    history.push(json!({"type":"user","content":[{"type":"text","text":"<user_query>fix tests</user_query>"}]}));
    let native = convert::<Grok, Codex>(&session(history, vec![])).unwrap();
    let common = Codex::to_common(&native).unwrap();
    assert_eq!(user_texts(&common), ["fix tests"]);
    assert!(common.body.iter().any(|m| {
        m.role == Role::Assistant
            && m.content
                .iter()
                .any(|b| matches!(b, Block::Text {text} if text.contains("implemented parser")))
    }));
}

#[test]
fn combined_environment_and_query_keeps_the_actual_prompt() {
    let grok = session(
        vec![
            json!({"type":"user","content":[{"type":"text","text":"<user_info>OS: linux</user_info>\n<rules><user_query>instructions</user_query></rules>\n<user_query>fix tests</user_query>"}]}),
        ],
        vec![],
    );
    assert_eq!(user_texts(&Grok::to_common(&grok).unwrap()), ["fix tests"]);
}

#[test]
fn legacy_context_and_query_suffixes_do_not_appear_as_user_text() {
    let grok = session(
        vec![
            json!({"type":"user","content":[{"type":"text","text":"<fork-context>Inherited session state</fork-context>"}]}),
            json!({"type":"user","content":[{"type":"text","text":"<rules>project instructions</rules>"}]}),
            json!({"type":"user","content":[{"type":"text","text":"<user_query>fix tests</user_query>\n<system-reminder>internal reminder</system-reminder>"}]}),
        ],
        vec![],
    );
    let common = Grok::to_common(&grok).unwrap();
    assert_eq!(user_texts(&common), ["fix tests"]);
    assert!(common.body.iter().any(|m| {
        m.role == Role::Assistant
            && m.content.iter().any(
                |b| matches!(b, Block::Text {text} if text.contains("Inherited session state")),
            )
    }));
}

#[test]
fn literal_tags_written_by_user_survive_when_display_log_identifies_them() {
    let actual = "Explain the <user_info>example</user_info> tag";
    let grok = session(
        vec![
            json!({"type":"user","content":[{"type":"text","text":format!("<user_query>{actual}</user_query>")} ]}),
        ],
        vec![
            json!({"timestamp":1_780_000_000,"params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":actual},"_meta":{"promptIndex":0}}}}),
        ],
    );
    assert_eq!(user_texts(&Grok::to_common(&grok).unwrap()), [actual]);
}

#[test]
fn image_only_prompt_is_not_dropped() {
    let grok = session(
        vec![json!({"type":"user","content":[]})],
        vec![
            json!({"timestamp":1_780_000_000,"params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"image","mimeType":"image/png","data":"UE5H"},"_meta":{"promptIndex":0}}}}),
        ],
    );
    let common = Grok::to_common(&grok).unwrap();
    assert_eq!(common.body.len(), 1);
    assert!(matches!(&common.body[0].content[..], [Block::Image { .. }]));
}

#[test]
fn unknown_synthetic_origin_is_preserved_without_claiming_human_authorship() {
    let grok = session(
        vec![
            json!({"type":"user","synthetic_reason":"future_origin","content":[{"type":"text","text":"Important state from a new runtime"}]}),
        ],
        vec![],
    );
    let common = Grok::to_common(&grok).unwrap();
    assert!(user_texts(&common).is_empty());
    assert_eq!(common.body[0].role, Role::Assistant);
}

#[test]
fn human_interjection_remains_user_authored() {
    let grok = session(
        vec![
            json!({"type":"user","synthetic_reason":"interjection","content":[{"type":"text","text":"Use the other file"}]}),
        ],
        vec![],
    );
    assert_eq!(
        user_texts(&Grok::to_common(&grok).unwrap()),
        ["Use the other file"]
    );
}

#[test]
fn a_compacted_session_matches_the_retained_prompt_in_the_full_display_log() {
    let updates = ["old prompt", "current prompt"].iter().enumerate().map(|(i, text)| {
        json!({"timestamp":1_780_000_000 + i,"params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":text},"_meta":{"promptIndex":i}}}})
    }).collect();
    let grok = session(
        vec![
            json!({"type":"user","synthetic_reason":"compaction_meta","content":[{"type":"text","text":"Completed old task"}]}),
            json!({"type":"user","content":[{"type":"text","text":"<user_query>current prompt</user_query>"}]}),
        ],
        updates,
    );
    let common = Grok::to_common(&grok).unwrap();
    assert_eq!(user_texts(&common), ["current prompt"]);
    assert_eq!(common.body[1].timestamp.timestamp(), 1_780_000_001);
}

#[test]
fn malformed_unclosed_tags_do_not_delete_the_message() {
    let raw = "<fork-context>unfinished text";
    let grok = session(vec![json!({"type":"user","content":raw})], vec![]);
    assert_eq!(user_texts(&Grok::to_common(&grok).unwrap()), [raw]);
}

#[test]
fn unwrapped_literal_examples_survive_without_a_display_log() {
    let raw = "Explain <user_info>example</user_info>";
    let grok = session(vec![json!({"type":"user","content":raw})], vec![]);
    assert_eq!(user_texts(&Grok::to_common(&grok).unwrap()), [raw]);
}

#[test]
fn preserved_runtime_context_keeps_its_role_in_every_destination() {
    use txcript::harness::{claude_code::ClaudeCode, cursor::Cursor, opencode::OpenCode};
    let grok = session(
        vec![
            json!({"type":"user","synthetic_reason":"compaction_meta","content":[{"type":"text","text":"Completed old task"}]}),
            json!({"type":"user","content":[{"type":"text","text":"<user_query>continue</user_query>"}]}),
        ],
        vec![],
    );
    for common in [
        ClaudeCode::to_common(&convert::<Grok, ClaudeCode>(&grok).unwrap()).unwrap(),
        Codex::to_common(&convert::<Grok, Codex>(&grok).unwrap()).unwrap(),
        Cursor::to_common(&convert::<Grok, Cursor>(&grok).unwrap()).unwrap(),
        OpenCode::to_common(&convert::<Grok, OpenCode>(&grok).unwrap()).unwrap(),
    ] {
        assert_eq!(user_texts(&common), ["continue"]);
        assert!(common.body.iter().any(|m| {
            m.role == Role::Assistant
                && m.content
                    .iter()
                    .any(|b| matches!(b,Block::Text{text} if text.contains("Completed old task")))
        }));
    }
}

#[test]
fn legacy_untagged_summary_is_not_a_user_prompt_when_display_log_identifies_the_real_turn() {
    let grok = session(
        vec![
            json!({"type":"user","content":[{"type":"text","text":"Previous conversation summary: changed two files."}]}),
            json!({"type":"user","content":[{"type":"text","text":"<user_query>continue</user_query>"}]}),
        ],
        vec![
            json!({"timestamp":1_780_000_000,"params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"continue"},"_meta":{"promptIndex":0}}}}),
        ],
    );
    let common = Codex::to_common(&convert::<Grok, Codex>(&grok).unwrap()).unwrap();
    assert_eq!(user_texts(&common), ["continue"]);
    assert!(common.body.iter().any(|m| {
        m.role == Role::Assistant
            && m.content
                .iter()
                .any(|b| matches!(b,Block::Text{text} if text.contains("changed two files")))
    }));
}

#[test]
fn wrapped_literal_examples_survive_when_the_display_log_is_missing() {
    let raw = "Explain <user_info>example</user_info>";
    let grok = session(
        vec![json!({"type":"user","content":format!("<user_query>{raw}</user_query>")})],
        vec![],
    );
    assert_eq!(user_texts(&Grok::to_common(&grok).unwrap()), [raw]);
}
