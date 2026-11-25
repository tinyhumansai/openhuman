//! Disabled-voice facade.
//!
//! Compiled only when the `voice` Cargo feature is OFF (see the gate in
//! [`super`]). It mirrors the subset of the real `voice` public surface that
//! other-gated callers depend on, with no-op / disabled-error
//! bodies so the crate still compiles, boots, and serves `/rpc` without the
//! voice domain.
//!
//! The signatures here MUST match the real ones exactly (return types included).
//! The disabled build
//! (`cargo check --no-default-features --features "<all-but-voice>"`) is the
//! only thing that catches drift — if a real signature changes, update the
//! mirror below until that build is green again.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::core::Outcome;

/// Error text returned by every disabled-path operation that must yield a
/// `Result`. Shared so callers/log-greps see one stable string.
const DISABLED_MSG: &str = "voice feature disabled at compile time";

// ---------------------------------------------------------------------------
// Provider factory surface (mirrors `factory::*` re-exported at the voice root)
// ---------------------------------------------------------------------------

/// Common STT result shape. Mirrors [`super::factory::SttResult`] (real build).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttResult {
    pub text: String,
    /// Lowercase provider id — kept for wire-shape parity with the real type.
    pub provider: String,
}

/// Speech-to-text provider abstraction. Object-safe (via `async_trait`) so
/// `Box<dyn SttProvider>` remains nameable at call sites; no concrete
/// implementation exists when voice is compiled out.
#[async_trait]
pub trait SttProvider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn transcribe(
        &self,
        config: &Config,
        audio_base64: &str,
        mime_type: Option<&str>,
        file_name: Option<&str>,
        language: Option<&str>,
    ) -> Result<Outcome<SttResult>, String>;

    #[cfg(test)]
    fn configured_model(&self) -> Option<&str> {
        None
    }
}

/// Resolve the effective STT provider string. With voice disabled the value is
/// never used to build a real provider; the config-independent default keeps
/// logging/telemetry callers seeing a sensible string.
pub fn effective_stt_provider(_config: &Config) -> String {
    "cloud".to_string()
}

/// Always errors: no STT provider can be constructed when voice is compiled
/// out. Callers `?`-propagate the error, so the boxed provider is never used.
pub async fn create_stt_provider(
    _provider: &str,
    _model: &str,
    _config: &Config,
) -> anyhow::Result<Box<dyn SttProvider>> {
    Err(anyhow::anyhow!(DISABLED_MSG))
}

// `live::ws::handle_live_voice_ws`: with voice off there are no live
// providers, so the upgraded socket is dropped immediately.
#[cfg(feature = "http-server")]
pub mod live {
    pub mod ws {
        use std::sync::Arc;

        use axum::extract::ws::WebSocket;

        use crate::config::Config;

        /// Drop the upgraded socket immediately — live voice is compiled out.
        pub async fn handle_live_voice_ws(
            _socket: WebSocket,
            _ctx: crate::core::runtime::DispatchCtx,
            _config: Arc<Config>,
        ) {
        }
    }
}
