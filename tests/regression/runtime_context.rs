//! Agents record setup blocks, reminders, notifications and timestamps as
//! user text. A Cursor chat moved to Grok opened with Cursor's
//! `<user_info>…</agent_skills>` block as the user's first message, and the
//! other readers carried the same kind of text. Only the typed prompt may
//! reach another agent.

use serde_json::{Value, json};
use txcript::common::{Block, Role};
use txcript::harness::{
    claude_code::ClaudeCode, codex::Codex, cursor::Cursor, grok::Grok, opencode::OpenCode,
};
use txcript::{Codec, Common, TextCodec, Transcript};

fn user_texts(common: &Transcript<Common>) -> Vec<String> {
    common
        .body
        .iter()
        .filter(|m| m.role == Role::User)
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            Block::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn round_trip<H: Codec>(common: &Transcript<Common>) -> Transcript<Common> {
    H::to_common(&H::from_common(common).unwrap()).unwrap()
}

fn assert_only_typed_prompts(common: &Transcript<Common>) {
    let expected = ["fix the build", "and the tests"];
    assert_eq!(user_texts(common), expected);
    for output in [
        round_trip::<ClaudeCode>(common),
        round_trip::<Codex>(common),
        round_trip::<Cursor>(common),
        round_trip::<Grok>(common),
        round_trip::<OpenCode>(common),
    ] {
        assert_eq!(user_texts(&output), expected);
    }
}

fn jsonl(lines: &[Value]) -> String {
    lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn claude_code_runtime_text_is_not_a_prompt() {
    let user = |uuid: &str, content: Value| json!({"type":"user","uuid":uuid,"timestamp":"2026-10-17T00:00:00Z","sessionId":"s","message":{"role":"user","content":content}});
    let source = jsonl(&[
        user(
            "u1",
            json!([
                {"type":"text","text":"<system-reminder>\nSkills are available.\n</system-reminder>"},
                {"type":"text","text":"fix the build"},
            ]),
        ),
        json!({"type":"assistant","uuid":"a1","timestamp":"2026-10-17T00:00:01Z","sessionId":"s","message":{"role":"assistant","content":[{"type":"text","text":"started"}]}}),
        user(
            "u2",
            json!(
                "<task-notification>\n<task-id>t1</task-id>\n<status>completed</status>\n</task-notification>"
            ),
        ),
        user(
            "u3",
            json!(
                "<timestamp>Saturday, Oct 17, 2026</timestamp>\n<user_query>\nand the tests\n</user_query>"
            ),
        ),
    ]);
    let common = ClaudeCode::to_common(&ClaudeCode::from_text(&source).unwrap()).unwrap();
    assert_only_typed_prompts(&common);
}

#[test]
fn codex_runtime_text_is_not_a_prompt() {
    let user = |text: &str| json!({"timestamp":"2026-10-17T00:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}});
    let source = jsonl(&[
        json!({"timestamp":"2026-10-17T00:00:00Z","type":"session_meta","payload":{"id":"s","cwd":"/work/repo"}}),
        user("<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>"),
        user("fix the build"),
        user("<turn_aborted>\nThe user interrupted the previous turn on purpose.\n</turn_aborted>"),
        user("and the tests"),
    ]);
    let common = Codex::to_common(&Codex::from_text(&source).unwrap()).unwrap();
    assert_only_typed_prompts(&common);
}
