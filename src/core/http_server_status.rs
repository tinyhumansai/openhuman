//! Compile-time visibility into the `http-server` gate.
//!
//! Deliberately **ungated**: unlike the rest of the transport surface, this
//! module is compiled in both feature states, because its whole purpose is to
//! report which state the binary ended up in. It mirrors
//! [`crate::openhuman::voice::VOICE_COMPILED_IN`] and
//! [`crate::openhuman::inference::INFERENCE_COMPILED_IN`].

/// Whether the real HTTP + Socket.IO server transport was compiled into this
/// binary.
///
/// Cargo features are per-crate and invisible to dependents' `#[cfg]`, so a
/// consumer that *requires* the transport (the desktop shell, which reaches the
/// core only over `http://127.0.0.1:<port>/rpc`) has no other way to detect
/// that it silently got a slim build with no listener — exactly the class of
/// silent drop that shipped `voice` broken from v0.58.19 (#4901).
///
/// The shell asserts this at compile time (`const _: () = assert!(...)` in
/// `app/src-tauri/src/lib.rs`), turning that silent runtime failure (every RPC
/// unreachable — the frontend can't talk to a core that never bound a socket)
/// into a build failure. When `false`, the direct `socketioxide` dependency is
/// dropped from the graph (verify with `cargo tree -i socketioxide`); `axum`
/// stays linked transitively via `tinychannels`, so only the gated HTTP +
/// Socket.IO transport surface — not `axum` itself — leaves the slim build.
pub const HTTP_SERVER_COMPILED_IN: bool = cfg!(feature = "http-server");

#[cfg(test)]
mod tests {
    use super::HTTP_SERVER_COMPILED_IN;

    /// Pins the constant to the gate rather than to a hardcoded value: the
    /// assertion inverts with the feature, so it holds for both the default
    /// build and the slim (`--no-default-features`) build.
    #[test]
    fn reports_the_compiled_gate_state() {
        assert_eq!(HTTP_SERVER_COMPILED_IN, cfg!(feature = "http-server"));
    }

    /// The default build ships the HTTP transport; this is the state the
    /// desktop app requires. Skipped when the slim build is under test.
    #[test]
    #[cfg(feature = "http-server")]
    fn is_true_when_the_http_server_feature_is_on() {
        assert!(HTTP_SERVER_COMPILED_IN);
    }

    /// The slim build must report honestly, otherwise the shell's const assert
    /// would pass against a listener-less core.
    #[test]
    #[cfg(not(feature = "http-server"))]
    fn is_false_when_the_http_server_feature_is_off() {
        assert!(!HTTP_SERVER_COMPILED_IN);
    }
}
