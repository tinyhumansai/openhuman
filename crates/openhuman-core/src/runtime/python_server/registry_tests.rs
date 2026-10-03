use super::*;

/// #5056: a fresh install (`Config::default()`) must not enable any
/// Python backend, so the runtime Python server never launches — and the
/// managed CPython interpreter is never speculatively downloaded — on a
/// default boot.
#[test]
fn enabled_backends_is_empty_by_default() {
    let config = Config::default();
    assert!(
        enabled_backends(&config).is_empty(),
        "default config must not enable any runtime Python backend"
    );
}

#[test]
fn registry_includes_kompress_when_enabled() {
    let mut config = Config::default();
    config.runtime_python.enabled = true;
    config.tokenjuice.ml_compression_enabled = true;
    assert_eq!(
        enabled_backends(&config),
        vec![RuntimePythonBackend::Kompress]
    );

    // Master runtime switch still gates everything.
    config.runtime_python.enabled = false;
    assert!(enabled_backends(&config).is_empty());
}
