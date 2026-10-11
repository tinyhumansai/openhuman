//! Global hotkey listener.
//!
//! `ActivationMode` and `HotkeyEvent` are shared wire vocabulary from the
//! TinyVoice bus contract. Native listeners and key parsing live inside the
//! loaded TinyVoice module.

pub use tinyvoice_bus::hotkey::*;
