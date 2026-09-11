//! Compile-time visibility into the `voice` gate.
//!
//! Deliberately **ungated**: unlike the rest of the domain, this module is
//! compiled in both feature states, because its whole purpose is to report
//! which state the binary ended up in. It is part of the always-compiled facade
//! described in [`super`], alongside `stub`.

/// Whether the real voice domain was compiled into this binary.
///
/// Cargo features are per-crate and invisible to dependents' `#[cfg]`, so a
/// consumer that *requires* voice (the desktop shell) has no other way to detect
/// that it silently got the stubbed build — which is exactly how #4901 shipped:
/// `app/src-tauri/Cargo.toml` set `default-features = false`, dropping the
/// default-ON `voice` feature, so every `openhuman.voice_*` controller went
/// unregistered and answered "unknown method" at runtime.
///
/// The shell asserts this at compile time (`const _: () = assert!(...)` in
/// `app/src-tauri/src/lib.rs`), turning that silent runtime failure into a build
/// failure.
pub const VOICE_COMPILED_IN: bool = cfg!(feature = "voice");

#[cfg(test)]
#[path = "compile_status_tests.rs"]
mod tests;
