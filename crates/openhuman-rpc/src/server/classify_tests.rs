use super::{classify_failure, is_wallet_not_configured_error, FailureDisposition};

#[test]
fn is_wallet_not_configured_error_matches_wallet_constant() {
    // The classifier keys off the wallet layer's exact "not configured"
    // message so wallet-backed RPCs stay out of Sentry.
    assert!(is_wallet_not_configured_error(
        crate::core_host::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE
    ));
}

#[test]
fn is_wallet_not_configured_error_is_coupled_to_the_wallet_constant() {
    // Drift guard: if the wallet wording changes without updating the shared
    // constant the classifier matches, this fails — preventing the noise from
    // silently returning to Sentry. Mirrors the param-validation prefix locks.
    assert_eq!(
        crate::core_host::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE,
        "wallet is not configured; run wallet setup first"
    );
}

#[test]
fn is_wallet_not_configured_error_does_not_match_other_errors() {
    // Other wallet/seed-derivation failures (decrypt, key derivation, locked
    // keychain) are real defects and must keep reaching Sentry.
    assert!(!is_wallet_not_configured_error(
        "wallet signer init: bad seed"
    ));
    assert!(!is_wallet_not_configured_error(
        "decrypt secret: kms timeout"
    ));
    assert!(!is_wallet_not_configured_error(""));
    // Substring-only must not qualify — exact equality is required.
    assert!(!is_wallet_not_configured_error(
        "rpc failed: wallet is not configured; run wallet setup first"
    ));
}

#[test]
fn classify_failure_expected_user_state_flag_wins() {
    // The envelope flag outranks every message-based rule, even a message
    // that would otherwise read as a session expiry.
    assert_eq!(
        classify_failure("Session expired", true),
        FailureDisposition::ExpectedUserState
    );
}

#[test]
fn classify_failure_routes_each_boundary_class() {
    // The backend's 401 shape is classified by the predicate the TinyHumans
    // layer installs at boot.
    openhuman_tinyhumans::observability::register();
    assert_eq!(
        classify_failure(
            crate::core_host::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE,
            false
        ),
        FailureDisposition::WalletNotConfigured
    );
    assert_eq!(
        classify_failure(
            &crate::core_host::core::params::unknown_param_message(
                "api_key",
                "config",
                "update_model_settings"
            ),
            false
        ),
        FailureDisposition::ParamValidation
    );
    assert_eq!(
        classify_failure("GET /teams failed (401 Unauthorized): {}", false),
        FailureDisposition::SessionExpired
    );
    assert_eq!(
        classify_failure(
            &format!(
                "{}totally.made.up.method",
                crate::core_host::core::dispatch::UNKNOWN_METHOD_PREFIX
            ),
            false
        ),
        FailureDisposition::UnknownMethod { probe: false }
    );
    assert_eq!(
        classify_failure("config.update_model_settings: store write failed", false),
        FailureDisposition::Unexpected
    );
}

#[test]
fn classify_failure_param_validation_outranks_session_expiry() {
    // A param description that happens to contain a session marker is still a
    // params mismatch: validation runs before the handler, so no backend call
    // could have expired anything.
    assert_eq!(
        classify_failure(
            &crate::core_host::core::params::missing_required_param_message(
                "token",
                "Session expired token"
            ),
            false
        ),
        FailureDisposition::ParamValidation
    );
}

#[test]
fn classify_failure_does_not_page_on_a_cached_module_load_failure() {
    // TAURI-RUST-117K et al.: a module that failed to load is cached and the
    // same failure is returned on every call. The load was reported once when
    // it resolved, so each RPC re-report must not be an error-level event.
    for message in [
        "module 'tinyconnectors' could not be loaded from the installer bundle: module \
         `windows-2022-x86_64` refused: module directory is writable by another user. Restart \
         the app after repairing the installation. This is terminal for the running process; \
         restart the app to try again.",
        "store_stats: backend failed: memory is unavailable: the memory module failed to load. \
         Restart the app to retry; the reason is in the log.",
    ] {
        assert_eq!(
            classify_failure(message, false),
            FailureDisposition::ModuleUnavailable,
            "{message}"
        );
    }
}
