//! Live captions and transcript workflows domain.
//!
//! Provides real-time captioning from microphone/system audio, transcript
//! persistence, and summary/meeting-note generation on completed transcripts.
//!
//! Log prefix: `[live_captions]`

pub mod diarize;
pub mod persist;
mod rpc;
mod schemas;
pub mod store;
pub mod translate;
pub mod types;
pub mod voice_profiles;

pub use schemas::{
    all_controller_schemas as all_live_captions_controller_schemas,
    all_registered_controllers as all_live_captions_registered_controllers,
    schemas as live_captions_schemas,
};
pub use types::{CaptionSegment, CaptionSource, Transcript, TranscriptState};
