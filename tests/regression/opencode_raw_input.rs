use serde_json::json;
use txcript::common::{Block, Role, Tool, ToolOutput};
use txcript::harness::opencode::OpenCode;
use txcript::{Codec, Transcript};

#[test]
fn codex_custom_tool_strings_are_valid_opencode_inputs_and_round_trip() {
    let source = Transcript::new(
        super::meta("custom-tool-source"),
        vec![super::msg(
            Role::Assistant,
            vec![Block::ToolUse {
                id: "custom-call".into(),
                tool: Tool::Raw {
                    tool_name: "functions.exec".into(),
                    input: json!("text(await tools.exec_command({cmd:'pwd'}));"),
                },
            }],
            0,
        )],
    );
    let target = OpenCode::from_common(&source).unwrap();
    let part = target.body.messages[0]
        .parts
        .iter()
        .find(|part| part["type"] == "tool")
        .unwrap();
    let input = &part["state"]["input"];
    assert!(input.is_object(), "OpenCode import rejects string inputs");
    assert_eq!(
        input["$txcriptRawInput"],
        json!("text(await tools.exec_command({cmd:'pwd'}));")
    );
    let round_trip = OpenCode::to_common(&target).unwrap();
    assert_eq!(round_trip.body[0].content, source.body[0].content);
}

#[test]
fn nonadjacent_parallel_results_and_reused_call_ids_keep_their_outputs() {
    let call = |id: &str| Block::ToolUse {
        id: id.into(),
        tool: Tool::Raw {
            tool_name: "test".into(),
            input: json!({}),
        },
    };
    let result = |id: &str, text: &str, is_error: bool| Block::ToolResult {
        tool_use_id: id.into(),
        content: ToolOutput::Text(text.into()),
        is_error,
    };
    let source = Transcript::new(
        super::meta("parallel-tools"),
        vec![
            super::msg(Role::Assistant, vec![call("read")], 0),
            super::msg(Role::Assistant, vec![call("shell")], 1),
            super::msg(Role::User, vec![result("read", "contents", false)], 2),
            super::msg(Role::User, vec![result("shell", "failed command", true)], 3),
            super::msg(Role::Assistant, vec![call("read")], 4),
            super::msg(Role::User, vec![result("read", "new contents", false)], 5),
            super::msg(Role::Assistant, vec![call("unfinished")], 6),
        ],
    );
    let target = OpenCode::from_common(&source).unwrap();
    let states: Vec<_> = target
        .body
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .filter(|p| p["type"] == "tool")
        .map(|p| &p["state"])
        .collect();
    assert_eq!(states[0]["output"], "contents");
    assert_eq!(states[1]["status"], "error");
    assert_eq!(states[1]["error"], "failed command");
    assert_eq!(states[2]["output"], "new contents");
    assert_eq!(states[3]["status"], "pending");
    let round = OpenCode::to_common(&target).unwrap();
    let results: Vec<_> = round
        .body
        .iter()
        .flat_map(|m| &m.content)
        .filter(|b| matches!(b, Block::ToolResult { .. }))
        .cloned()
        .collect();
    assert_eq!(
        results,
        vec![
            result("read", "contents", false),
            result("shell", "failed command", true),
            result("read", "new contents", false)
        ]
    );
}
