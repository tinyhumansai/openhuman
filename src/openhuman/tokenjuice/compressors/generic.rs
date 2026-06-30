//! Generic line-oriented fallback compressor.
//!
//! Used by the router when a specialised compressor declines (or the ML text
//! compressor is unavailable). It runs the rule engine's `generic/fallback`
//! head/tail summariser — but **only for command output**. For domain-tool
//! payloads (no derived command/argv) it declines, preserving the long-standing
//! guard that large structured tool results must reach the downstream
//! progressive-disclosure handoff rather than being blindly head/tail clamped.

use async_trait::async_trait;

use super::Compressor;
use crate::openhuman::tokenjuice::compressors::log::compress_command_fallback;
use crate::openhuman::tokenjuice::types::{
    CompressInput, CompressOptions, CompressOutput, CompressorKind,
};

pub struct GenericCompressor;

#[async_trait]
impl Compressor for GenericCompressor {
    fn kind(&self) -> CompressorKind {
        CompressorKind::Generic
    }

    async fn compress(
        &self,
        input: &CompressInput<'_>,
        opts: &CompressOptions,
    ) -> Option<CompressOutput> {
        let has_command =
            input.command.is_some() || input.argv.as_ref().is_some_and(|a| !a.is_empty());
        if !has_command {
            // Domain-tool payload — decline rather than blind-truncate.
            return None;
        }
        compress_command_fallback(input, opts)
    }
}
