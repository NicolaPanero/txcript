use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use txcript::common::Role;
use txcript::common::{Block, Message, ToolOutput};
use txcript::harness::{
    claude_code::{self, ClaudeStore},
    codex::{self, CodexStore},
    cursor::CursorStore,
    grok::GrokStore,
    opencode::{self, OpenCodeStore},
};
use txcript::{Codec, Common, CropError, HarnessId, Span, Store, Transcript};

const ENGINE: &str = "0.14.4-fork.19";
const MAX_SOURCE: u64 = 100 * 1024 * 1024;
/// A long session's file keeps every compacted window; only the last one is
/// carried, and `MAX_SOURCE` bounds that.
const MAX_SOURCE_FILE: u64 = 1024 * 1024 * 1024;
/// About 140k tokens of JSON: what a target agent can take in one prompt and
/// still compact.
const MAX_CONTEXT_BYTES: usize = 480_000;
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

fn read<S>(
    store: &S,
    reference: &S::Ref,
    active: impl FnOnce(&mut Transcript<S::H>),
) -> Result<Transcript<Common>, String>
where
    S: Store,
    S::H: Codec,
{
    let mut native = store.load(reference).map_err(|_| "source_unreadable")?;
    active(&mut native);
    S::H::to_common(&native).map_err(|_| "source_conversion_failed".into())
}

fn read_source(req: &Request, root: &Path, reference: &Path) -> Result<Transcript<Common>, String> {
    read_agent(
        &req.source_agent,
        &req.source_session_id,
        root,
        reference,
        true,
    )
}

/// `active` keeps only what the agent still had after its last compaction.
fn read_agent(
    agent: &str,
    session_id: &str,
    root: &Path,
    reference: &Path,
    active: bool,
) -> Result<Transcript<Common>, String> {
    match agent {
        "claude_code" => read(&ClaudeStore::new(root), &reference.to_path_buf(), |t| {
            if active {
                t.body = claude_code::active_context(&t.body);
            }
        }),
        "codex" => read(&CodexStore::new(root), &reference.to_path_buf(), |t| {
            if active {
                t.body = codex::active_context(&t.body);
            }
        }),
        "cursor" => read(&CursorStore::new(root), &reference.to_path_buf(), |_| {}),
        "grok" => read(&GrokStore::new(root), &reference.to_path_buf(), |_| {}),
        "opencode" => read(
            &OpenCodeStore::new(reference),
            &session_id.to_string(),
            |t| {
                if active {
                    t.body = opencode::active_context(&t.body);
                }
            },
        ),
        _ => Err("source_not_verified".into()),
    }
}

fn validate_jsonl(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|_| "source_unreadable")?;
    if file.metadata().map_err(|_| "source_unreadable")?.len() > MAX_SOURCE_FILE {
        return Err("source_too_large".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "source_unreadable")?;
    if bytes.len() as u64 > MAX_SOURCE_FILE {
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

fn caller_reference(
    reference: &Path,
    canonical_root: &Path,
    requested_root: &Path,
) -> Result<PathBuf, String> {
    let relative = reference
        .strip_prefix(canonical_root)
        .map_err(|_| "target_validation_failed")?;
    if relative.as_os_str().is_empty() {
        Ok(requested_root.to_path_buf())
    } else {
        Ok(requested_root.join(relative))
    }
}

/// What a message costs in the target's prompt: its text, tool input and
/// output, not the provider bookkeeping the canonical form carries.
fn encoded_len(message: &Message) -> usize {
    fn json_len<T: serde::Serialize>(value: &T) -> usize {
        serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
    }
    message
        .content
        .iter()
        .map(|block| match block {
            Block::Text { text } | Block::Thinking { text, .. } => text.len(),
            Block::ToolUse { tool, .. } => json_len(tool),
            Block::ToolResult { content, .. } => json_len(content),
            Block::Image { .. } | Block::Artifact { .. } => 2_000,
        })
        .sum()
}

/// Keeps the newest messages that fit `MAX_CONTEXT_BYTES`, never separating a
/// tool call from its result, and opens with a user turn so every target
/// accepts it. Returns whether anything was dropped.
fn fit_context(common: &mut Transcript<Common>) -> Result<bool, String> {
    let sizes: Vec<usize> = common.body.iter().map(encoded_len).collect();
    let mut start = common.body.len();
    let mut total = 0;
    while start > 0 && total + sizes[start - 1] <= MAX_CONTEXT_BYTES {
        start -= 1;
        total += sizes[start];
    }
    let trimmed = start > 0;
    if trimmed {
        let len = common.body.len();
        let start = start.min(len - 1);
        let cropped = (start..len).find_map(|from| match common.crop(&Span(from..len)) {
            Ok(cropped) => Some(Ok(cropped)),
            Err(CropError::SplitToolPair { .. }) => None,
            Err(_) => Some(Err("source_conversion_failed".to_string())),
        });
        *common = cropped.ok_or("source_conversion_failed")??;
    }
    if common
        .body
        .first()
        .is_some_and(|m| m.role == Role::Assistant)
    {
        let timestamp = common.body[0].timestamp;
        common.body.insert(
            0,
            Message {
                role: Role::User,
                content: vec![Block::Text {
                    text: "Earlier parts of this conversation were summarized or omitted when it was moved to this agent. Continue from what follows.".into(),
                }],
                timestamp,
                model: None,
                stop_reason: None,
                usage: None,
            },
        );
    }
    Ok(trimmed)
}

const AGENT_MARKUP_TAGS: [&str; 7] = [
    "environment_context",
    "system-reminder",
    "turn_aborted",
    "user_action",
    "permissions",
    "collaboration_mode",
    "user_instructions",
];

/// Text an agent injects under the user role to drive its own app: hidden
/// there, and noise to any other agent.
fn is_agent_markup(text: &str) -> bool {
    let text = text.trim();
    if text == "</image>"
        || (text.starts_with("<image ") || text == "<image>")
            && text.ends_with('>')
            && !text.contains('\n')
    {
        return true;
    }
    let Some(rest) = text.strip_prefix('<') else {
        return false;
    };
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    !name.is_empty()
        && (name.starts_with("external_")
            || name.starts_with("command-")
            || name.starts_with("local-command-")
            || AGENT_MARKUP_TAGS.contains(&name.as_str()))
        && text.ends_with(&format!("</{name}>"))
}

/// Codex wraps a prompt that has attachments in a file listing; the prompt is
/// what follows its "## My request" heading.
fn attachment_prompt(text: &str) -> Option<&str> {
    if !text
        .trim_start()
        .starts_with("# Files mentioned by the user:")
    {
        return None;
    }
    let heading = text.rfind("\n## My request")?;
    let body = &text[heading + 1..];
    Some(body.split_once('\n').map_or("", |(_, rest)| rest))
}

/// Drops agent markup from user turns and unwraps attachment listings, so the
/// target sees the prompt and its images as the user gave them.
fn clean_user_markup(messages: &mut Vec<Message>) {
    for message in messages.iter_mut().filter(|m| m.role == Role::User) {
        message.content.retain_mut(|block| match block {
            Block::Text { text } => {
                if is_agent_markup(text) {
                    return false;
                }
                if let Some(prompt) = attachment_prompt(text) {
                    *text = prompt.to_string();
                }
                !text.trim().is_empty()
            }
            _ => true,
        });
    }
    messages.retain(|m| !m.content.is_empty());
}

fn conversion_warnings(target: HarnessId, messages: &[Message]) -> Vec<&'static str> {
    let mut warnings =
        vec!["Runtime permissions, MCP configuration and credentials are not transferred."];
    if matches!(target, HarnessId::ClaudeCode | HarnessId::OpenCode)
        && messages.iter().flat_map(|m| &m.content).any(|b| {
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
    if messages
        .iter()
        .flat_map(|m| &m.content)
        .any(|b| matches!(b, Block::Thinking { .. }))
    {
        warnings.push(
            "Provider-specific reasoning signatures and encrypted reasoning are not portable.",
        );
    }
    if target == HarnessId::Cursor
        && messages.iter().flat_map(|m| &m.content).any(|block| {
            matches!(
                block,
                Block::ToolUse {
                    tool: txcript::common::Tool::Raw { .. } | txcript::common::Tool::Command { .. },
                    ..
                }
            )
        })
    {
        warnings.push("Foreign tools are shown as recorded imported calls; their original arguments and results are retained. No tools or MCP servers are configured or executed by conversion.");
    }
    warnings
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
    clean_user_markup(&mut common.body);
    let trimmed = fit_context(&mut common)?;
    let mut warnings = conversion_warnings(target, &common.body);
    if trimmed {
        warnings.push("Older messages were left out so the chat fits the target's context window.");
    }
    common.meta.id = if target == HarnessId::Codex {
        uuid::Uuid::now_v7()
    } else {
        uuid::Uuid::new_v4()
    }
    .to_string();
    common.meta.timestamp = Utc::now();
    let caller_cwd = if req.cwd.is_absolute() {
        req.cwd.clone()
    } else {
        cwd.clone()
    };
    common.meta.cwd = Some(caller_cwd.to_string_lossy().into_owned());
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
    let written_path = Path::new(&reference)
        .canonicalize()
        .map_err(|_| "target_validation_failed")?;
    let requested_root = if req.target_root.is_absolute() {
        &req.target_root
    } else {
        &target_root
    };
    let reference = caller_reference(&written_path, &target_root, requested_root)?;
    Ok(
        json!({"protocolVersion":1,"engineVersion":ENGINE,"targetAgent":req.target_agent,
        "targetSessionId":written.id,"reference":reference,"cwd":caller_cwd,"messageCount":messages,"warnings":warnings,"toolIdMap":tool_id_map}),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(clippy::struct_field_names)]
struct HistoryRequest {
    source_agent: String,
    source_session_id: String,
    source_root: PathBuf,
    source_reference: PathBuf,
    /// Page before this message index; the first page ends where the
    /// transfer's carried context starts.
    #[serde(default)]
    before: Option<usize>,
}

const HISTORY_TEXT_BYTES: usize = 20_000;
const HISTORY_TOOL_BYTES: usize = 2_000;
/// Below the host's 1 MiB cap on helper output, images included.
const HISTORY_PAGE_BYTES: usize = 700_000;
/// Image data sent per message; images past it are listed but not sent, so
/// one message cannot fill a page alone.
const HISTORY_IMAGE_BYTES: usize = 600_000;

fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn tool_summary(tool: &txcript::common::Tool) -> (String, String) {
    let value = serde_json::to_value(tool).unwrap_or(Value::Null);
    let name = match value.get("name").and_then(Value::as_str) {
        Some("Raw") => value
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("Tool"),
        Some(name) => name,
        None => "Tool",
    }
    .to_string();
    let detail = ["command", "file_path"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map_or_else(
            || {
                let input = value.get("input").unwrap_or(&value);
                clip(&input.to_string(), HISTORY_TOOL_BYTES)
            },
            |text| clip(text, HISTORY_TOOL_BYTES),
        );
    (name, detail)
}

fn history_message(
    index: usize,
    message: &Message,
    results: &BTreeMap<String, (String, bool)>,
) -> Option<Value> {
    let mut blocks = Vec::new();
    let mut image_budget = HISTORY_IMAGE_BYTES;
    for block in &message.content {
        match block {
            Block::Text { text } if !text.trim().is_empty() => {
                blocks.push(json!({"type":"text","text":clip(text, HISTORY_TEXT_BYTES)}));
            }
            Block::Thinking { text, .. } if !text.trim().is_empty() => {
                blocks.push(json!({"type":"thinking","text":clip(text, HISTORY_TEXT_BYTES)}));
            }
            Block::ToolUse { id, tool } => {
                let (name, input) = tool_summary(tool);
                let (output, is_error) = results.get(id).cloned().unwrap_or_default();
                blocks.push(json!({"type":"tool","id":id,"name":name,"input":input,"output":output,"isError":is_error}));
            }
            Block::Image { source } if source.source_type == "base64" => {
                blocks.push(if source.data.len() <= image_budget {
                    image_budget -= source.data.len();
                    json!({"type":"image","mediaType":source.media_type,"data":source.data})
                } else {
                    json!({"type":"image","mediaType":source.media_type,"omitted":true})
                });
            }
            _ => {}
        }
    }
    (!blocks.is_empty()).then(|| {
        json!({
            "index": index,
            "role": if message.role == Role::User { "user" } else { "assistant" },
            "timestampMs": message.timestamp.timestamp_millis(),
            "blocks": blocks,
        })
    })
}

/// One page of the messages a transfer left out, newest first in paging and
/// chronological in the page, shaped for display. `nextBefore` pages further
/// back; indices count from the start of the session, which only grows at
/// its end, so they stay valid while the source goes on.
fn history(req: &HistoryRequest) -> Result<Value, String> {
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
    let read = |active| {
        read_agent(
            &req.source_agent,
            &req.source_session_id,
            &root,
            &reference,
            active,
        )
        .map(|mut t| {
            clean_user_markup(&mut t.body);
            t
        })
    };
    let full = read(false)?;
    let mut carried = read(true)?;
    fit_context(&mut carried)?;
    let carried_len = carried
        .body
        .iter()
        .filter(|m| {
            !matches!(m.content.first(), Some(Block::Text { text }) if text.starts_with("Earlier parts of this conversation"))
        })
        .count();
    let earlier = full.body.len().saturating_sub(carried_len);
    let end = req.before.map_or(earlier, |before| before.min(earlier));
    let mut results = BTreeMap::new();
    for message in &full.body[..earlier] {
        for block in &message.content {
            if let Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } = block
            {
                let text = match content {
                    ToolOutput::Text(text) => text.clone(),
                    ToolOutput::Json(value) => value.to_string(),
                };
                results.insert(
                    tool_use_id.clone(),
                    (clip(&text, HISTORY_TOOL_BYTES), *is_error),
                );
            }
        }
    }
    let mut page = Vec::new();
    let mut size = 0;
    let mut start = end;
    while start > 0 {
        let Some(message) = history_message(start - 1, &full.body[start - 1], &results) else {
            start -= 1;
            continue;
        };
        let next = message.to_string().len();
        if !page.is_empty() && size + next > HISTORY_PAGE_BYTES {
            break;
        }
        size += next;
        page.push(message);
        start -= 1;
    }
    page.reverse();
    let next_before = (start > 0).then_some(start);
    Ok(json!({
        "protocolVersion": 1,
        "engineVersion": ENGINE,
        "truncated": next_before.is_some(),
        "nextBefore": next_before,
        "messages": page,
    }))
}

fn run() -> Result<(), String> {
    if std::env::args().nth(1).as_deref() == Some("capabilities") {
        println!(
            "{}",
            json!({"protocolVersion":1,"engineVersion":ENGINE,"conversionOnly":true,"verifiedAdapters":ADAPTERS,"declaredAdapters":ADAPTERS})
        );
        return Ok(());
    }
    let history_mode = std::env::args().nth(1).as_deref() == Some("history");
    let mut input = String::new();
    io::stdin()
        .take(64 * 1024 + 1)
        .read_to_string(&mut input)
        .map_err(|_| "input_unreadable")?;
    if input.len() > 64 * 1024 {
        return Err("input_too_large".into());
    }
    if history_mode {
        let request = serde_json::from_str(&input).map_err(|_| "input_invalid")?;
        println!("{}", history(&request)?);
        return Ok(());
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
    #[test]
    fn drops_agent_markup_and_unwraps_attachment_prompts() {
        let at = chrono::Utc::now();
        let user = |content: Vec<Block>| Message {
            role: Role::User,
            content,
            timestamp: at,
            model: None,
            stop_reason: None,
            usage: None,
        };
        let text = |t: &str| Block::Text { text: t.into() };
        let image = Block::Image {
            source: txcript::common::ImageSource {
                source_type: "base64".into(),
                media_type: "image/png".into(),
                data: "UE5H".into(),
            },
        };
        let mut messages = vec![
            user(vec![text(
                "<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>",
            )]),
            user(vec![
                text(
                    "\n# Files mentioned by the user:\n\n## a.png: /tmp/a.png\nImage attachment: true\n\n## My request:\nwhat is this?",
                ),
                text("<image name=[Image #1] path=\"/tmp/a.png\">"),
                image.clone(),
                text("</image>"),
            ]),
            user(vec![text("keep <b>this</b>")]),
        ];
        clean_user_markup(&mut messages);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, vec![text("what is this?"), image]);
        assert_eq!(messages[1].content, vec![text("keep <b>this</b>")]);
    }

    use super::*;
    use txcript::common::{Role, Tool};
    #[test]
    fn cursor_foreign_tool_replay_is_explained_without_claiming_configured_tools() {
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![Block::ToolUse {
                id: "fixture".into(),
                tool: Tool::Raw {
                    tool_name: "mcp__fixture__lookup".into(),
                    input: json!({"query":"fixture"}),
                },
            }],
            timestamp: Utc::now(),
            model: None,
            stop_reason: None,
            usage: None,
        }];
        let cursor = conversion_warnings(HarnessId::Cursor, &messages);
        assert!(
            cursor
                .iter()
                .any(|warning| warning.contains("recorded imported calls"))
        );
        let codex = conversion_warnings(HarnessId::Codex, &messages);
        assert!(
            !codex
                .iter()
                .any(|warning| warning.contains("recorded imported calls"))
        );
    }
    #[test]
    fn maps_verified_target_to_callers_path_spelling_without_allowing_escape() {
        let actual = PathBuf::from("actual");
        let requested = PathBuf::from("caller");
        let reference = actual.join("nested").join("session.jsonl");
        assert_eq!(
            caller_reference(&reference, &actual, &requested).unwrap(),
            requested.join("nested").join("session.jsonl")
        );
        assert_eq!(
            caller_reference(&actual, &actual, &requested)
                .unwrap()
                .as_os_str(),
            requested.as_os_str()
        );
        assert_eq!(
            caller_reference(&PathBuf::from("outside/session.jsonl"), &actual, &requested)
                .unwrap_err(),
            "target_validation_failed"
        );
    }

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
