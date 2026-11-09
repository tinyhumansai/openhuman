//! OpenHuman-owned speech bindings around TinyInference's local runtime.

mod speech;

pub use speech::{transcribe, transcribe_bytes, transcribe_with_prompt, tts};
