//! Context that agents record as user text although the user never typed it:
//! environment setup, reminders, notifications and timestamps. Readers drop
//! it so another agent does not receive it as the user's prompt.

use crate::common::{Block, Message, Role};

const RUNTIME_TAGS: &[&str] = &[
    // Cursor's setup block at the start of a chat.
    "user_info",
    "git_status",
    "agent_transcripts",
    "agent_skills",
    "available_subagent_types",
    "available_subagent_models",
    "dynamic_tool_catalog",
    // Reminders and notices from Claude Code, Cursor, Grok and OpenCode.
    "system-reminder",
    "system_reminder",
    "system_notification",
    "task-notification",
    "side-sessions-event",
    // Codex.
    "environment_context",
    "turn_aborted",
    "external_codex_apps_open_page",
    "guardian_tool_descriptions",
    // Claude desktop.
    "artifact-view-context",
    "timestamp",
];

/// `text` after one leading runtime element, if it starts with one.
fn after_runtime_element(text: &str) -> Option<&str> {
    let name = text.strip_prefix('<')?;
    let tag = RUNTIME_TAGS.iter().find(|tag| {
        name.strip_prefix(**tag)
            .is_some_and(|rest| rest.starts_with(['>', ' ', '\n']))
    })?;
    let close = format!("</{tag}>");
    let end = text.find(&close)? + close.len();
    Some(&text[end..])
}

/// What the user typed in `text`. Runtime elements count only at the start,
/// so a prompt that quotes one keeps it.
fn typed_text(text: &str) -> Option<String> {
    let mut rest = text.trim_start();
    let mut stripped = false;
    while let Some(after) = after_runtime_element(rest) {
        rest = after.trim_start();
        stripped = true;
    }
    if !stripped {
        return Some(text.to_string());
    }
    let rest = rest.trim_end();
    let query = rest
        .strip_prefix("<user_query>")
        .and_then(|q| q.strip_suffix("</user_query>"))
        .map_or(rest, str::trim);
    (!query.is_empty()).then(|| query.to_string())
}

/// `messages` without runtime text in user turns. A user turn left with no
/// content is dropped; tool results and images stay.
pub(crate) fn drop_runtime_context(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .filter_map(|mut message| {
            if message.role != Role::User {
                return Some(message);
            }
            let had_content = !message.content.is_empty();
            message.content = message
                .content
                .into_iter()
                .filter_map(|block| match block {
                    Block::Text { text } => typed_text(&text).map(|text| Block::Text { text }),
                    other => Some(other),
                })
                .collect();
            (!had_content || !message.content.is_empty()).then_some(message)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_runtime_elements_and_keeps_the_typed_prompt() {
        assert_eq!(
            typed_text("<user_info>\nOS\n</user_info>\n<agent_skills>\n</agent_skills>"),
            None
        );
        assert_eq!(
            typed_text("<system_notification source=\"goal\">\nContinue\n</system_notification>"),
            None
        );
        assert_eq!(
            typed_text("<timestamp>Monday</timestamp>\n<user_query>\nhello\n</user_query>"),
            Some("hello".into())
        );
        assert_eq!(
            typed_text("<system-reminder>\nskills\n</system-reminder>\nfix the test"),
            Some("fix the test".into())
        );
    }

    #[test]
    fn keeps_prompts_that_quote_or_resemble_runtime_elements() {
        for text in [
            "Explain <system-reminder>x</system-reminder> here",
            "<timestamps> are not a tag",
            "<user_query>kept as typed</user_query>",
            "<system-reminder> without a closing tag",
            "<bash-input>ls</bash-input>",
        ] {
            assert_eq!(typed_text(text), Some(text.into()));
        }
    }
}
