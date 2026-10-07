use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use txcript::common::{Block, Message, ToolOutput};
use txcript::harness::{
    claude_code::ClaudeStore, codex::CodexStore, cursor::CursorStore, grok::GrokStore,
    opencode::OpenCodeStore,
};
use txcript::{Codec, Common, HarnessId, Store, Transcript};

const ENGINE: &str = "0.14.4-fork.6";
const MAX_SOURCE: u64 = 100 * 1024 * 1024;
const ADAPTERS: [&str; 5] = ["claude_code", "codex", "cursor", "grok", "opencode"];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    source_agent: String,
    source_session_id: String,
    source_root: PathBuf,
    source_reference: PathBuf,
    target_agent: String,
    target_root: PathBuf,
    cwd: PathBuf,
}

fn read<S>(store: &S, reference: &S::Ref) -> Result<Transcript<Common>, String>
where
    S: Store,
    S::H: Codec,
{
    let native = store.load(reference).map_err(|_| "source_unreadable")?;
    S::H::to_common(&native).map_err(|_| "source_conversion_failed".into())
}

fn read_source(req: &Request, root: &Path, reference: &Path) -> Result<Transcript<Common>, String> {
    match req.source_agent.as_str() {
        "claude_code" => read(&ClaudeStore::new(root), &reference.to_path_buf()),
        "codex" => read(&CodexStore::new(root), &reference.to_path_buf()),
        "cursor" => read(&CursorStore::new(root), &reference.to_path_buf()),
        "grok" => read(&GrokStore::new(root), &reference.to_path_buf()),
        "opencode" => read(&OpenCodeStore::new(reference), &req.source_session_id),
        _ => Err("source_not_verified".into()),
    }
}

fn validate_jsonl(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|_| "source_unreadable")?;
    if file.metadata().map_err(|_| "source_unreadable")?.len() > MAX_SOURCE {
        return Err("source_too_large".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "source_unreadable")?;
    if bytes.len() as u64 > MAX_SOURCE {
        return Err("source_too_large".into());
    }
    if std::str::from_utf8(&bytes)
        .map_err(|_| "source_not_utf8")?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .any(|line| serde_json::from_str::<Value>(line).is_err())
    {
        return Err("source_incomplete".into());
    }
    Ok(())
}

fn target_harness(agent: &str) -> Result<HarnessId, String> {
    match agent {
        "claude_code" => Ok(HarnessId::ClaudeCode),
        "codex" => Ok(HarnessId::Codex),
        "cursor" => Ok(HarnessId::Cursor),
        "grok" => Ok(HarnessId::Grok),
        "opencode" => Ok(HarnessId::OpenCode),
        _ => Err("target_not_verified".into()),
    }
}

fn validated_source(req: &Request) -> Result<(PathBuf, Transcript<Common>), String> {
    let cwd = req.cwd.canonicalize().map_err(|_| "cwd_unavailable")?;
    let root = req
        .source_root
        .canonicalize()
        .map_err(|_| "source_root_unavailable")?;
    let reference = req
        .source_reference
        .canonicalize()
        .map_err(|_| "source_unavailable")?;
    if !reference.starts_with(&root) {
        return Err("source_outside_store".into());
    }
    match req.source_agent.as_str() {
        "claude_code" | "codex" => validate_jsonl(&reference)?,
        "grok" => {
            for name in ["chat_history.jsonl", "updates.jsonl", "events.jsonl"] {
                let path = reference.join(name);
                if path.exists() {
                    let path = path.canonicalize().map_err(|_| "source_unreadable")?;
                    if !path.starts_with(&reference) {
                        return Err("source_outside_store".into());
                    }
                    validate_jsonl(&path)?;
                }
            }
        }
        _ => {}
    }
    let common = read_source(req, &root, &reference)?;
    if common.meta.id != req.source_session_id {
        return Err("source_id_mismatch".into());
    }
    if common.body.is_empty() {
        return Err("source_empty".into());
    }
    if serde_json::to_vec(&common.body)
        .map_err(|_| "source_conversion_failed")?
        .len() as u64
        > MAX_SOURCE
    {
        return Err("source_too_large".into());
    }
    let source_cwd = common.meta.cwd.as_deref().ok_or("source_cwd_missing")?;
    if Path::new(source_cwd)
        .canonicalize()
        .map_err(|_| "source_cwd_unavailable")?
        != cwd
    {
        return Err("source_cwd_mismatch".into());
    }
    if common != read_source(req, &root, &reference)? {
        return Err("source_changed_during_read".into());
    }
    Ok((cwd, common))
}

/// Cursor stores one time for a whole chat; targets that sort by time would
/// reorder its turns, so each message is kept after the previous one.
fn order_timestamps(messages: &mut [Message]) {
    for i in 1..messages.len() {
        let previous = messages[i - 1].timestamp;
        if messages[i].timestamp <= previous {
            messages[i].timestamp = previous + chrono::Duration::milliseconds(1);
        }
    }
}

fn normalize_tool_ids(messages: &mut [Message]) -> BTreeMap<String, String> {
    let mut occupied: HashSet<String> = messages
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|b| match b {
            Block::ToolUse { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    let mut aliases = BTreeMap::new();
    for message in messages {
        for block in &mut message.content {
            let id = match block {
                Block::ToolUse { id, .. } => id,
                Block::ToolResult { tool_use_id, .. } => tool_use_id,
                _ => continue,
            };
            if id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                let replacement = aliases.entry(id.clone()).or_insert_with(|| {
                    let mut counter = 0_u64;
                    loop {
                        let key = format!("{id}:{counter}");
                        let candidate = format!(
                            "call_{}",
                            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes()).simple()
                        );
                        if occupied.insert(candidate.clone()) {
                            break candidate;
                        }
                        counter += 1;
                    }
                });
                id.clone_from(replacement);
            }
        }
    }
    aliases
}

fn convert(req: &Request) -> Result<Value, String> {
    let (cwd, mut common) = validated_source(req)?;
    let target = target_harness(&req.target_agent)?;
    if req.source_agent == req.target_agent {
        return Err("same_agent".into());
    }
    let target_root = req
        .target_root
        .canonicalize()
        .map_err(|_| "target_root_unavailable")?;
    if target == HarnessId::OpenCode {
        let store = OpenCodeStore::default_db().ok_or("target_root_unavailable")?;
        if store
            .db_path
            .canonicalize()
            .map_err(|_| "target_root_unavailable")?
            != target_root
        {
            return Err("target_store_mismatch".into());
        }
    }
    let mut warnings =
        vec!["Runtime permissions, MCP configuration and credentials are not transferred."];
    if matches!(target, HarnessId::ClaudeCode | HarnessId::OpenCode)
        && common.body.iter().flat_map(|m| &m.content).any(|b| {
            matches!(
                b,
                Block::ToolResult {
                    content: ToolOutput::Json(_),
                    ..
                }
            )
        })
    {
        warnings.push("Structured tool output is preserved as JSON text for this target.");
    }
    if common
        .body
        .iter()
        .flat_map(|m| &m.content)
        .any(|b| matches!(b, Block::Thinking { .. }))
    {
        warnings.push(
            "Provider-specific reasoning signatures and encrypted reasoning are not portable.",
        );
    }
    common.meta.id = if target == HarnessId::Codex {
        uuid::Uuid::now_v7()
    } else {
        uuid::Uuid::new_v4()
    }
    .to_string();
    common.meta.timestamp = Utc::now();
    common.meta.cwd = Some(cwd.to_string_lossy().into_owned());
    common.meta.model = None;
    common.meta.lineage = None;
    let tool_id_map = normalize_tool_ids(&mut common.body);
    order_timestamps(&mut common.body);
    if !tool_id_map.is_empty() {
        warnings.push("Tool call identifiers were normalized for the target; call/result pairing is retained.");
    }
    let messages = common.body.len();
    let written = txcript::local::write(
        target,
        &common,
        if target == HarnessId::OpenCode {
            None
        } else {
            Some(&target_root)
        },
    )
    .map_err(|_| "target_write_failed")?;
    let reference: String = if target == HarnessId::OpenCode {
        target_root.to_string_lossy().into_owned()
    } else {
        serde_json::from_str(&written.location).unwrap_or_else(|_| written.location.clone())
    };
    let verify = Request {
        source_agent: req.target_agent.clone(),
        source_session_id: written.id.clone(),
        source_root: target_root.clone(),
        source_reference: reference.clone().into(),
        target_agent: String::new(),
        target_root: target_root.clone(),
        cwd: cwd.clone(),
    };
    let restored = read_source(&verify, &target_root, Path::new(&reference))?;
    if restored.meta.id != written.id
        || restored.body.is_empty()
        || restored.meta.cwd.as_deref() != common.meta.cwd.as_deref()
    {
        return Err("target_validation_failed".into());
    }
    Ok(
        json!({"protocolVersion":1,"engineVersion":ENGINE,"targetAgent":req.target_agent,
        "targetSessionId":written.id,"reference":reference,"cwd":cwd,"messageCount":messages,"warnings":warnings,"toolIdMap":tool_id_map}),
    )
}

fn run() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() == Some("capabilities") {
        println!(
            "{}",
            json!({"protocolVersion":1,"engineVersion":ENGINE,"conversionOnly":true,"verifiedAdapters":ADAPTERS,"declaredAdapters":ADAPTERS})
        );
        return Ok(());
    }
    let mut input = String::new();
    io::stdin()
        .take(64 * 1024 + 1)
        .read_to_string(&mut input)
        .map_err(|_| "input_unreadable")?;
    if input.len() > 64 * 1024 {
        return Err("input_too_large".into());
    }
    let request = serde_json::from_str(&input).map_err(|_| "input_invalid")?;
    println!("{}", convert(&request)?);
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(code) => {
            eprintln!("{}", json!({"error":code}));
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use txcript::common::{Role, Tool};
    #[test]
    fn aliases_invalid_ids_without_collisions_and_keeps_results_paired() {
        let invalid = "functions.shell:call.with.a.long.identifier.that.exceeds.the.targets.sixty.four.characters";
        let collision = format!(
            "call_{}",
            uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_URL,
                format!("{invalid}:0").as_bytes()
            )
            .simple()
        );
        let call = |id: &str| Block::ToolUse {
            id: id.into(),
            tool: Tool::Raw {
                tool_name: "Shell".into(),
                input: json!({"command":"printf fixture"}),
            },
        };
        let result = |id: &str| Block::ToolResult {
            tool_use_id: id.into(),
            content: ToolOutput::Text("fixture".into()),
            is_error: false,
        };
        let mut messages = vec![Message {
            role: Role::Assistant,
            content: vec![call(invalid), call(&collision), result(invalid)],
            timestamp: Utc::now(),
            model: None,
            stop_reason: None,
            usage: None,
        }];
        let aliases = normalize_tool_ids(&mut messages);
        let mapped = &aliases[invalid];
        assert_ne!(mapped, &collision);
        assert!(mapped.len() <= 64);
        assert_eq!(
            messages[0].content,
            vec![call(mapped), call(&collision), result(mapped)]
        );
        assert!(normalize_tool_ids(&mut messages).is_empty());
    }
}
