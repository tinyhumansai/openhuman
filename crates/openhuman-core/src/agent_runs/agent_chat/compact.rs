//! Compacting a thread's stored conversation with no model turn.

use crate::agent::session_host::{CompactOptions, CompactOutcome};
use crate::config::Config;

use super::session_turn::{apply_turn_overrides, build_turn_agent, AgentChatTarget};

/// Compact `thread_id`'s stored conversation on `target`'s session now, so
/// its next turn resends a summary instead of the full history.
///
/// `model_override`, `temperature` and `route` are applied to `config` as a
/// turn applies them, and the session is built and bound exactly as
/// [`agent_chat_for`](super::agent_chat_for) builds a turn's, so the compaction addresses the
/// transcript that turn would resume and holds the same per-session turn
/// lock. See [`OpenHumanSessionHost::compact_bound_history`]
/// (crate::agent::OpenHumanSessionHost::compact_bound_history).
///
/// Returned boxed and `#[inline(never)]` for the reason `agent_chat_for` is:
/// the future is awaited from other crates.
///
/// # Errors
///
/// When `thread_id` is blank, the session cannot be built, or the compaction
/// fails.
#[inline(never)]
pub fn compact_thread_for<'a>(
    config: &'a mut Config,
    target: AgentChatTarget<'a>,
    thread_id: &'a str,
    model_override: Option<String>,
    temperature: Option<f64>,
    route: Option<crate::config::schema::EphemeralRoute>,
    options: CompactOptions,
) -> futures::future::BoxFuture<'a, anyhow::Result<CompactOutcome>> {
    Box::pin(async move {
        apply_turn_overrides(config, model_override, temperature, route);
        let thread_id = thread_id.trim();
        if thread_id.is_empty() {
            anyhow::bail!("compact_thread needs a thread id");
        }
        // The context a turn without a bound host environment uses: the
        // caller's ambient one.
        let ctx = crate::core::runtime::DispatchCtx::from_weak(None);
        let mut session = build_turn_agent(&ctx, config, &target, Some(thread_id), None, None)
            .map_err(anyhow::Error::msg)?;
        session.set_thread_id(Some(thread_id));
        tracing::debug!(thread_id, ?options, "[compact_thread] session bound");
        session.compact_bound_history(options).await
    })
}
