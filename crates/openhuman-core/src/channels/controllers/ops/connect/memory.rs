//! Clearing a channel's memory on disconnect.

use crate::config::Config;

/// Forgets every conversation memory v2 stored from this channel (items
/// tagged `channel:<channel_id>`), and reports how many items went.
///
/// The count is reported to the caller as `memory_chunks_deleted` (the
/// disconnect reply's field name predates memory v2 and is kept as a wire
/// contract); it now counts stored conversation items. Memory off forgets
/// nothing and reports zero, which is true: an engine-less host stored
/// nothing for the channel.
pub(super) async fn clear_channel_memory(
    config: &Config,
    channel_id: &str,
) -> anyhow::Result<usize> {
    let forgotten = crate::memory::conversations::forget_channel(config, channel_id)
        .await
        .map_err(|e| anyhow::anyhow!("clear_channel_memory: {e} (code {})", e.code()))?;
    tracing::debug!(
        channel = %channel_id,
        forgotten,
        "[channels][memory] cleared channel conversations"
    );
    Ok(forgotten)
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
