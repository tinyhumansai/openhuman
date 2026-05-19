//! Telegram remote-control slash commands (phase 1: `/status`, `/sessions`, `/new`).

use super::session_store::{with_store, TelegramChatBinding};
use crate::openhuman::channels::context::{
    clear_sender_history, conversation_history_key, ChannelRouteSelection, ChannelRuntimeContext,
};
use crate::openhuman::channels::traits::ChannelMessage;
use crate::openhuman::memory::conversations::{self, ConversationThread, CreateConversationThread};

const LOG_PREFIX: &str = "[telegram-remote]";

pub(crate) const TELEGRAM_CMD_STATUS: &str = "/status";
pub(crate) const TELEGRAM_CMD_SESSIONS: &str = "/sessions";
pub(crate) const TELEGRAM_CMD_NEW: &str = "/new";
pub(crate) const TELEGRAM_CMD_HELP: &str = "/help";

const SESSIONS_LIST_LIMIT: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelegramRemoteCommand {
    Status,
    Sessions,
    New,
    Help,
}

pub(crate) fn parse_telegram_remote_command(content: &str) -> Option<TelegramRemoteCommand> {
    let trimmed = content.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let command_token = trimmed.split_whitespace().next()?;
    let base = command_token
        .split('@')
        .next()
        .unwrap_or(command_token)
        .to_ascii_lowercase();
    match base.as_str() {
        TELEGRAM_CMD_STATUS => Some(TelegramRemoteCommand::Status),
        TELEGRAM_CMD_SESSIONS => Some(TelegramRemoteCommand::Sessions),
        TELEGRAM_CMD_NEW => Some(TelegramRemoteCommand::New),
        TELEGRAM_CMD_HELP => Some(TelegramRemoteCommand::Help),
        _ => None,
    }
}

pub(crate) async fn build_remote_command_response(
    ctx: &ChannelRuntimeContext,
    msg: &ChannelMessage,
    command: TelegramRemoteCommand,
) -> String {
    tracing::debug!(
        "{LOG_PREFIX} command={command:?} reply_target={} sender={}",
        msg.reply_target,
        msg.sender
    );
    match command {
        TelegramRemoteCommand::Status => build_status_response(ctx, msg).await,
        TelegramRemoteCommand::Sessions => build_sessions_response(ctx, msg).await,
        TelegramRemoteCommand::New => build_new_session_response(ctx, msg).await,
        TelegramRemoteCommand::Help => build_help_response(),
    }
}

fn build_help_response() -> String {
    [
        "OpenHuman Telegram remote control (phase 1):",
        "",
        &format!("• `{TELEGRAM_CMD_STATUS}` — active thread, model, and turn state"),
        &format!("• `{TELEGRAM_CMD_SESSIONS}` — recent conversation threads"),
        &format!("• `{TELEGRAM_CMD_NEW}` — start a fresh thread for this chat"),
        &format!("• `{TELEGRAM_CMD_HELP}` — this message"),
        "",
        "Model routing: `/model`, `/models` (same as before).",
    ]
    .join("\n")
}

fn route_for_sender(ctx: &ChannelRuntimeContext, sender_key: &str) -> ChannelRouteSelection {
    ctx.route_overrides
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(sender_key)
        .cloned()
        .unwrap_or_else(|| ChannelRouteSelection {
            provider: ctx.default_provider.as_str().to_string(),
            model: ctx.model.as_str().to_string(),
        })
}

async fn build_status_response(ctx: &ChannelRuntimeContext, msg: &ChannelMessage) -> String {
    let sender_key = conversation_history_key(msg);
    let route = route_for_sender(ctx, &sender_key);
    let history_len = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&sender_key)
        .map(|h| h.len())
        .unwrap_or(0);

    let workspace = ctx.workspace_dir.as_path();
    let (binding, busy) = match with_store(workspace, |store| {
        Ok((
            store.binding(&msg.reply_target).cloned(),
            store.is_busy(&msg.reply_target),
        ))
    }) {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!("{LOG_PREFIX} status: session store error: {error}");
            (None, false)
        }
    };

    let thread_line = match binding {
        Some(TelegramChatBinding { thread_id, .. }) => {
            let title = lookup_thread_title(workspace, &thread_id)
                .await
                .unwrap_or_else(|| thread_id.clone());
            format!("Thread: `{title}` (`{thread_id}`)")
        }
        None => "Thread: _(none — send `/new` to bind a thread)_".to_string(),
    };

    let turn_state = if busy { "in progress ⏳" } else { "idle" };

    format!(
        "**Status**\n\
         {thread_line}\n\
         Provider: `{provider}`\n\
         Model: `{model}`\n\
         In-memory turns: {history_len}\n\
         Turn: {turn_state}",
        provider = route.provider,
        model = route.model,
    )
}

async fn build_sessions_response(ctx: &ChannelRuntimeContext, msg: &ChannelMessage) -> String {
    let workspace = ctx.workspace_dir.as_path();
    let active_thread_id = with_store(workspace, |store| {
        Ok(store
            .binding(&msg.reply_target)
            .map(|b| b.thread_id.clone()))
    })
    .ok()
    .flatten();

    let threads = match conversations::list_threads(workspace.to_path_buf()) {
        Ok(list) => list,
        Err(error) => {
            tracing::warn!("{LOG_PREFIX} sessions: list_threads failed: {error}");
            return format!("Could not list sessions: {error}");
        }
    };

    if threads.is_empty() {
        return "No conversation threads yet. Send `/new` to create one.".to_string();
    }

    let mut sorted = threads;
    sorted.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));

    let mut lines = vec![
        "**Recent sessions**".to_string(),
        format!("Showing up to {SESSIONS_LIST_LIMIT} threads:"),
        String::new(),
    ];

    for thread in sorted.into_iter().take(SESSIONS_LIST_LIMIT) {
        lines.push(format_session_line(&thread, active_thread_id.as_deref()));
    }

    lines.join("\n")
}

fn format_session_line(thread: &ConversationThread, active_id: Option<&str>) -> String {
    let marker = if active_id == Some(thread.id.as_str()) {
        "→ "
    } else {
        "  "
    };
    let title = if thread.title.trim().is_empty() {
        thread.id.as_str()
    } else {
        thread.title.as_str()
    };
    format!(
        "{marker}`{title}` — {count} msgs (id: `{id}`)",
        count = thread.message_count,
        id = thread.id,
    )
}

async fn build_new_session_response(ctx: &ChannelRuntimeContext, msg: &ChannelMessage) -> String {
    let workspace = ctx.workspace_dir.as_path();
    let sender_key = conversation_history_key(msg);
    let thread_id = format!("thread-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now();
    let title = format!(
        "Telegram {} {}",
        now.format("%b %-d"),
        now.format("%-I:%M %p")
    );
    let created_at = now.to_rfc3339();

    if let Err(error) = conversations::ensure_thread(
        workspace.to_path_buf(),
        CreateConversationThread {
            id: thread_id.clone(),
            title: title.clone(),
            created_at,
            parent_thread_id: None,
            labels: Some(vec!["telegram".to_string(), "remote".to_string()]),
        },
    ) {
        tracing::warn!("{LOG_PREFIX} new: ensure_thread failed: {error}");
        return format!("Failed to create session: {error}");
    }

    clear_sender_history(ctx, &sender_key);

    if let Err(error) = with_store(workspace, |store| {
        store.set_binding(&msg.reply_target, thread_id.clone(), sender_key.clone());
        Ok(())
    }) {
        tracing::warn!("{LOG_PREFIX} new: persist binding failed: {error}");
        return format!(
            "Created thread `{thread_id}` but failed to persist Telegram binding: {error}"
        );
    }

    crate::openhuman::channels::providers::web::invalidate_thread_sessions(&thread_id).await;

    tracing::info!(
        "{LOG_PREFIX} new session thread_id={thread_id} reply_target={} sender_key={sender_key}",
        msg.reply_target
    );

    format!(
        "Started new session **{title}**.\n\
         Thread id: `{thread_id}`\n\
         In-memory channel history cleared for this chat."
    )
}

async fn lookup_thread_title(workspace: &std::path::Path, thread_id: &str) -> Option<String> {
    let threads = conversations::list_threads(workspace.to_path_buf()).ok()?;
    threads
        .into_iter()
        .find(|t| t.id == thread_id)
        .map(|t| t.title)
}

#[cfg(test)]
#[path = "remote_control_tests.rs"]
mod tests;
