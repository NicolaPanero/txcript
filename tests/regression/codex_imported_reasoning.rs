use serde_json::json;
use txcript::common::{Block, Role};
use txcript::harness::{claude_code::ClaudeCode, codex::Codex};
use txcript::{Codec, TextCodec};

#[test]
fn imported_reasoning_is_replayable_text_and_keeps_its_block_type() {
    let source = json!({
        "type": "assistant",
        "uuid": "reply",
        "sessionId": "imported-reasoning",
        "cwd": "/work/repo",
        "timestamp": "2026-01-02T03:04:05Z",
        "message": {
            "role": "assistant",
            "content": [
                {"type": "thinking", "thinking": "Inspect the source 日本語", "signature": "foreign-signature"},
                {"type": "text", "text": "The complete original reply"}
            ]
        }
    });
    let claude = ClaudeCode::from_text(&source.to_string()).unwrap();
    let common = ClaudeCode::to_common(&claude).unwrap();
    let native = Codex::from_common(&common).unwrap();
    let serialized = Codex::to_text(&native).unwrap();
    let reloaded = Codex::from_text(&serialized).unwrap();
    let items: Vec<_> = reloaded
        .body
        .iter()
        .filter(|line| line.kind == "response_item")
        .map(|line| &line.payload)
        .collect();

    assert!(items.iter().all(|item| item["type"] == "message"));
    assert_eq!(items[0]["role"], "assistant");
    assert_eq!(items[0]["phase"], "commentary");
    assert_eq!(items[0]["content"][0]["text"], "Inspect the source 日本語");
    assert!(!serialized.contains("foreign-signature"));

    let back = Codex::to_common(&reloaded).unwrap();
    let blocks: Vec<_> = back
        .body
        .iter()
        .flat_map(|message| &message.content)
        .collect();
    assert!(
        back.body
            .iter()
            .all(|message| message.role == Role::Assistant)
    );
    assert_eq!(blocks.len(), 2);
    assert!(
        matches!(blocks[0], Block::Thinking { text, .. } if text == "Inspect the source 日本語")
    );
    assert!(matches!(blocks[1], Block::Text { text } if text == "The complete original reply"));
}
