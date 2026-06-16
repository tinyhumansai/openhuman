//! Controller schemas and RPC handler dispatch for the voice domain.

mod handlers;
mod helpers;
mod params;
mod registry;

// Re-export the public API that callers outside this module use.
pub use registry::{all_voice_controller_schemas, all_voice_registered_controllers, voice_schemas};

// ---------------------------------------------------------------------------
// Internal re-exports used by the test companion file.
// ---------------------------------------------------------------------------

#[cfg(test)]
use handlers::{
    handle_overlay_stt_notify, handle_voice_server_start, handle_voice_server_status,
    handle_voice_server_stop,
};
#[cfg(test)]
use helpers::{
    deserialize_params, generate_silent_wav, to_json, validate_stt_provider, validate_tts_provider,
};
#[cfg(test)]
use params::{
    OverlaySttNotifyParams, OverlaySttState, ReplySynthesizeParams, SetProvidersParams,
    SttDispatchParams, TranscribeBytesParams, TranscribeParams, TtsDispatchParams, TtsParams,
    VoiceListModelsParams, VoiceProviderCredUpdate, VoiceTestProviderParams,
    VoiceUpdateProviderSettingsParams,
};

#[cfg(test)]
use crate::rpc::RpcOutcome;
#[cfg(test)]
use serde_json::Map;
#[cfg(test)]
use serde_json::Value;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "../schemas_tests.rs"]
mod tests;
