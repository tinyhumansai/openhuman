//! Debug-build seams for raw integration coverage of dispatch helpers.

use super::*;

pub fn build_channel_context_block_for_test(msg: &traits::ChannelMessage) -> String {
    build_channel_context_block(msg)
}

pub fn select_acknowledgment_reaction_for_test(content: &str) -> &'static str {
    select_acknowledgment_reaction(content)
}
