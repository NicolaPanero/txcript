use serde_json::{Value, json};
use txcript::common::{Block, ToolOutput};
use txcript::harness::{claude_code::ClaudeCode, codex::Codex};
use txcript::{Codec, TextCodec};

#[test]
fn codex_structured_outputs_survive_claude_conversion() {
    for kind in ["custom_tool_call_output", "function_call_output"] {
        for output in [
            json!([
                {"type":"input_text","text":"Script completed with exit code 0."},
                {"type":"input_text","text":"{\"output\":\"/work/repo\\nstatus: codex\"}"}
            ]),
            json!({"future_output":{"text":"keep this too"}}),
            json!("legacy text output"),
        ] {
            let call_kind = if kind == "custom_tool_call_output" {
                "custom_tool_call"
            } else {
                "function_call"
            };
            let lines = [
                json!({"type":"session_meta","payload":{"id":"source","cwd":"/work/repo","timestamp":"2026-01-02T03:04:05Z"}}),
                json!({"type":"response_item","payload":{"type":call_kind,"call_id":"call-real-format","name":"exec_command","input":"pwd","arguments":"{\"cmd\":\"pwd\"}"}}),
                json!({"type":"response_item","payload":{"type":kind,"call_id":"call-real-format","output":output}}),
            ];
            let text = lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            let native = Codex::from_text(&text).unwrap();
            let common = Codex::to_common(&native).unwrap();
            let results = common
                .body
                .iter()
                .flat_map(|m| &m.content)
                .filter_map(|b| {
                    if let Block::ToolResult { content, .. } = b {
                        Some(content)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            let expected = if let Some(s) = output.as_str() {
                ToolOutput::Text(s.into())
            } else {
                ToolOutput::Json(output.clone())
            };
            assert_eq!(results, [&expected]);
            let target = ClaudeCode::from_common(&common).unwrap();
            let saved = ClaudeCode::to_text(&target).unwrap();
            let target_lines = saved
                .lines()
                .map(|s| serde_json::from_str::<Value>(s).unwrap())
                .collect::<Vec<_>>();
            let result = target_lines
                .iter()
                .flat_map(|l| l["message"]["content"].as_array().into_iter().flatten())
                .find(|b| b["type"] == "tool_result")
                .unwrap();
            let expected_text = output
                .as_str()
                .map_or_else(|| output.to_string(), str::to_owned);
            assert_eq!(result["content"], expected_text);
            let roundtrip = Codex::to_text(&native).unwrap();
            let reloaded = roundtrip
                .lines()
                .map(|s| serde_json::from_str::<Value>(s).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(reloaded, lines);
        }
    }
}
