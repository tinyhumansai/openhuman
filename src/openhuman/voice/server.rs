//! Standalone voice server — hotkey → record → transcribe → insert text.
//!
//! Can run as part of the core process or independently via the CLI.
//! The server listens for a configurable hotkey, records audio from the
//! microphone, transcribes via the configured STT engine, and inserts the result into the
//! active text field.

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
include!("server_part_01.rs");
include!("server_part_02.rs");
