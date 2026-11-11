//! Narrow helpers exposed to the raw-coverage integration target.

/// Build the host-owned channel context block for a bus message.
pub fn build_channel_context_block_for_test(msg: &tinychannels_bus::ChannelMessage) -> String {
    crate::channels::runtime::dispatch::build_channel_context_block(msg)
}
