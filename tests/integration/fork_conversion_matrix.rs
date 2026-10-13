#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use serde_json::json;
use txcript::common::{Block, ImageSource, Message, Meta, Role, Tool, ToolOutput};
use txcript::harness::{
    claude_code::ClaudeCode, codex::Codex, cursor::Cursor, grok::Grok, opencode::OpenCode,
};
use txcript::{Codec, Common, TextCodec, Transcript};

fn codec<H: Codec + TextCodec>(common: &Transcript<Common>) -> Transcript<Common> {
    let native = H::from_common(common).unwrap();
    H::to_common(&H::from_text(&H::to_text(&native).unwrap()).unwrap()).unwrap()
}

fn convert(agent: &str, common: &Transcript<Common>) -> Transcript<Common> {
    match agent {
        "claude" => codec::<ClaudeCode>(common),
        "codex" => codec::<Codex>(common),
        "cursor" => codec::<Cursor>(common),
        "grok" => codec::<Grok>(common),
        "opencode" => codec::<OpenCode>(common),
        _ => panic!("unknown fixture agent"),
    }
}

fn message(role: Role, content: Vec<Block>, step: i64) -> Message {
    Message {
        role,
        content,
        timestamp: chrono::DateTime::from_timestamp(1_780_000_000 + step, 0).unwrap(),
        model: None,
        stop_reason: None,
        usage: None,
    }
}

fn tools() -> [Tool; 5] {
    [
        Tool::Read {
            file_path: "/repo/a.rs".into(),
            offset: None,
            limit: Some(20),
        },
        Tool::Write {
            file_path: "/repo/b.rs".into(),
            content: "fn main() {}".into(),
        },
        Tool::Edit {
            file_path: "/repo/a.rs".into(),
            old_string: "old".into(),
            new_string: "new".into(),
            replace_all: false,
        },
        Tool::Bash {
            command: "cargo test".into(),
            workdir: None,
            timeout_ms: None,
            description: None,
            run_in_background: false,
        },
        Tool::Raw {
            tool_name: "mcp__custom__lookup".into(),
            input: json!({"query":"日本語","flags":[true,null]}),
        },
    ]
}

fn sample() -> Transcript<Common> {
    let meta = Meta {
        id: "fixture-session".into(),
        timestamp: chrono::DateTime::from_timestamp(1_780_000_000, 0).unwrap(),
        cwd: Some("/repo".into()),
        git_branch: None,
        title: Some("Compatibility".into()),
        cli_version: None,
        model: None,
        lineage: None,
    };
    let mut body = vec![
        message(
            Role::User,
            vec![Block::Text {
                text: "Fix the parser 日本語\nDo not invent prompts".into(),
            }],
            0,
        ),
        message(
            Role::Assistant,
            vec![
                Block::Thinking {
                    text: "Check exact roles and tools".into(),
                    signature: None,
                    encrypted: None,
                },
                Block::Text {
                    text: "Inspecting files".into(),
                },
            ],
            1,
        ),
    ];
    for (i, tool) in tools().into_iter().enumerate() {
        let id = format!("call-{i}");
        body.push(message(
            Role::Assistant,
            vec![Block::ToolUse {
                id: id.clone(),
                tool,
            }],
            2 + i64::try_from(i).unwrap() * 2,
        ));
        body.push(message(
            Role::User,
            vec![Block::ToolResult {
                tool_use_id: id,
                content: if i == 4 {
                    ToolOutput::Json(json!({"items":["a","b"],"count":2}))
                } else {
                    ToolOutput::Text(format!("result-{i}"))
                },
                is_error: i == 3,
            }],
            3 + i64::try_from(i).unwrap() * 2,
        ));
    }
    body.push(message(
        Role::User,
        vec![
            Block::Text {
                text: "Check this attachment".into(),
            },
            Block::Image {
                source: ImageSource {
                    source_type: "base64".into(),
                    media_type: "image/png".into(),
                    data: "UE5H".into(),
                },
            },
        ],
        20,
    ));
    body.push(message(
        Role::Assistant,
        vec![Block::Text {
            text: "Final answer\nTools have not been executed by conversion".into(),
        }],
        21,
    ));
    Transcript::new(meta, body)
}

fn content_signature(common: &Transcript<Common>) -> Vec<String> {
    common
        .body
        .iter()
        .flat_map(|message| {
            message.content.iter().map(move |block| match block {
                Block::Text { text } => format!("{:?}/text:{text}", message.role),
                Block::Thinking { text, .. } => format!("thinking:{text}"),
                Block::ToolUse { id, tool } => {
                    let (name, input) = tool.to_canonical();
                    format!("use:{id}:{name}:{input}")
                }
                Block::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    let text = match content {
                        ToolOutput::Text(s) => s.clone(),
                        ToolOutput::Json(v) => v.to_string(),
                    };
                    format!("result:{tool_use_id}:{is_error}:{text}")
                }
                Block::Image { source } => format!("image:{}:{}", source.media_type, source.data),
                other @ Block::Artifact { .. } => format!("{other:?}"),
            })
        })
        .collect()
}

#[test]
fn every_supported_direction_preserves_roles_tools_errors_images_and_json() {
    let expected = content_signature(&sample());
    for source in ["claude", "codex", "cursor", "grok", "opencode"] {
        let parsed = convert(source, &sample());
        assert_eq!(content_signature(&parsed), expected, "source {source}");
        for target in ["claude", "codex", "cursor", "grok", "opencode"] {
            let output = convert(target, &parsed);
            assert_eq!(content_signature(&output), expected, "{source} -> {target}");
        }
    }
}

#[test]
fn user_authored_context_tag_examples_are_not_injected_environment() {
    let mut common = sample();
    common.body = vec![message(
        Role::User,
        vec![Block::Text {
            text: "Explain <user_info>example</user_info> and <fork-context>text</fork-context>"
                .into(),
        }],
        0,
    )];
    let expected = content_signature(&common);
    for source in ["claude", "codex", "cursor", "grok", "opencode"] {
        let parsed = convert(source, &common);
        assert_eq!(content_signature(&parsed), expected, "source {source}");
        for target in ["claude", "codex", "cursor", "grok", "opencode"] {
            assert_eq!(
                content_signature(&convert(target, &parsed)),
                expected,
                "{source} -> {target}"
            );
        }
    }
}
