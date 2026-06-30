//! ML plain-text compressor ("Kompress") — trait slot.
//!
//! Plain text has no structural skeleton to exploit, so high-quality
//! compression needs a learned model (Headroom uses ModernBERT token
//! classification to drop low-salience spans). That path runs the `kompress`
//! backend of the shared `runtime_python_server` and is **opt-in** behind the
//! `tokenjuice.ml_compression_enabled` config flag.
//!
//! This module is the [`Compressor`] slot; it delegates to
//! [`crate::openhuman::tokenjuice::ml`]. Whenever the flag is off or the Python
//! runtime is unavailable, `compress` declines so the router falls back to the
//! generic compressor — never an error in the agent loop.

use async_trait::async_trait;

use super::Compressor;
use crate::openhuman::tokenjuice::types::{
    CompressInput, CompressOptions, CompressOutput, CompressorKind,
};

pub struct MlTextCompressor;

#[async_trait]
impl Compressor for MlTextCompressor {
    fn kind(&self) -> CompressorKind {
        CompressorKind::MlText
    }

    async fn compress(
        &self,
        input: &CompressInput<'_>,
        opts: &CompressOptions,
    ) -> Option<CompressOutput> {
        if !opts.ml_text_enabled {
            return None;
        }
        match crate::openhuman::tokenjuice::ml::compress(input.content, opts).await {
            Ok(Some(text)) if text.len() < input.content.len() => {
                Some(CompressOutput::lossy(text, CompressorKind::MlText))
            }
            Ok(_) => None,
            Err(e) => {
                log::debug!("[tokenjuice][ml] unavailable, falling back: {e:#}");
                None
            }
        }
    }
}
