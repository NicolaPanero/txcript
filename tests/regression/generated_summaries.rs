use serde_json::json;
use txcript::common::{Block, Role};
use txcript::harness::{
    antigravity::Antigravity, claude_code::ClaudeCode, codex::Codex, cursor::Cursor, grok::Grok,
    opencode::OpenCode,
};
use txcript::{Codec, Common, TextCodec, Transcript};

const SUMMARY: &str = "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\nSummary:\n1. Synthetic fixture: parser implemented; tests are next.";

fn round_trip<H: Codec>(common: &Transcript<Common>) -> Transcript<Common> {
    H::to_common(&H::from_common(common).unwrap()).unwrap()
}

fn all_targets(common: &Transcript<Common>) -> [Transcript<Common>; 6] {
    [
        round_trip::<ClaudeCode>(common),
        round_trip::<Codex>(common),
        round_trip::<Cursor>(common),
        round_trip::<Grok>(common),
        round_trip::<OpenCode>(common),
        round_trip::<Antigravity>(common),
    ]
}

#[test]
fn claude_generated_summary_never_becomes_a_human_prompt_after_transfer() {
    let source = ClaudeCode::from_text(
        &json!({
            "type":"user", "uuid":"summary-fixture", "isCompactSummary":true,
            "timestamp":"2026-10-14T00:00:00Z", "sessionId":"fixture-session",
            "message":{"role":"user","content":SUMMARY},
        })
        .to_string(),
    )
    .unwrap();
    let common = ClaudeCode::to_common(&source).unwrap();
    assert_eq!(common.body.len(), 1);
    assert_eq!(common.body[0].role, Role::Assistant);
    for output in all_targets(&common) {
        assert_eq!(output.body.len(), 1);
        assert_eq!(output.body[0].role, Role::Assistant);
        assert!(
            output.body[0]
                .content
                .iter()
                .any(|block| matches!(block,Block::Text{text} if text==SUMMARY))
        );
    }
}

#[test]
fn an_identical_text_pasted_by_a_human_keeps_its_user_role() {
    for marker in [None, Some(false)] {
        let mut record = json!({"type":"user","uuid":"literal-fixture","message":{"role":"user","content":SUMMARY}});
        if let Some(marker) = marker {
            record["isCompactSummary"] = json!(marker);
        }
        let common =
            ClaudeCode::to_common(&ClaudeCode::from_text(&record.to_string()).unwrap()).unwrap();
        for output in all_targets(&common) {
            assert_eq!(output.body[0].role, Role::User);
            assert!(
                output.body[0]
                    .content
                    .iter()
                    .any(|block| matches!(block,Block::Text{text} if text==SUMMARY))
            );
        }
    }
}

#[test]
fn grok_compaction_context_stays_assistant_context_for_every_target() {
    let source=Grok::from_text(&json!({"chat_history":[{
        "type":"user","synthetic_reason":"compaction_meta","content":[{"type":"text","text":SUMMARY}],
    }]}).to_string()).unwrap();
    let common = Grok::to_common(&source).unwrap();
    assert_eq!(common.body[0].role, Role::Assistant);
    for output in all_targets(&common) {
        assert_eq!(output.body[0].role, Role::Assistant);
    }
}
