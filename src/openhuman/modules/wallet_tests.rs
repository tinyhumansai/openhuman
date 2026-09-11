//! Tests for the wallet module client.
//!
//! Nothing here loads a module. What is testable without one is the part that
//! decides what a caller does next: how a bus failure is classified, that the
//! unavailable path is reached without a broker, and — most importantly — that
//! the local signing produces what the module asked for. The round trips
//! themselves are covered where they can be honest: `tinywallet`'s own loader
//! E2E, which drives a real module over a real broker.

use tinywallet_bus::wire::TransactionSpec;
use tinywallet_bus::Chain;

use super::{classify, WalletCallError};
use crate::openhuman::config::Config;

/// The BIP-39 test vector mnemonic. Never use it for real funds.
const VECTOR: &str = "abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon about";

/// A config with modules enabled but nothing fetchable.
fn offline_config() -> Config {
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config
}

/// A bus failure carrying `name`.
fn failure(name: &str) -> tinybus::Error {
    tinybus::Error::MethodFailed {
        name: name.to_string(),
        message: "something went wrong".to_string(),
    }
}

/// The phrase and path a confidential call carries.
///
/// No key is derived here any more: the module does that. This is the request,
/// not the secret it produces.
fn evm_signing_secret() -> tinywallet_bus::wire::SecretMaterial {
    tinywallet_bus::wire::SecretMaterial {
        mnemonic: VECTOR.to_string(),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        chain: Chain::Evm,
    }
}

#[test]
fn an_invalid_input_is_reported_as_something_the_caller_can_fix() {
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinywallet.Error.InvalidInput")),
        WalletCallError::InvalidInput(_)
    ));
}

#[test]
fn a_build_failure_is_not_an_input_error() {
    // Telling a caller its transaction was wrong when the encoder broke points
    // at the wrong fix.
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinywallet.Error.BuildFailed")),
        WalletCallError::Failed(_)
    ));
}

#[test]
fn an_unsupported_chain_reads_as_unavailable_not_invalid() {
    // Defensive: the current module never emits this name — an unrecognised
    // transaction shape comes back as `InvalidInput` instead. The arm exists
    // so a future module that does distinguish "chain not compiled in" from
    // "bad request" is classified as a missing capability rather than as the
    // caller's fault.
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinywallet.Error.UnsupportedChain")),
        WalletCallError::Unavailable(_)
    ));
}

#[test]
fn an_unrecognised_wire_name_is_a_failure_not_an_input_error() {
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinywallet.Error.SomethingNewer")),
        WalletCallError::Failed(_)
    ));
}

#[test]
fn a_missing_module_reads_as_unavailable() {
    assert!(matches!(
        classify(&failure("ai.tinyhumans.tinybus.Error.ModuleUnavailable")),
        WalletCallError::Unavailable(_)
    ));
}

#[test]
fn every_error_renders_as_its_message() {
    assert_eq!(
        WalletCallError::InvalidInput("bad recipient".to_string()).to_string(),
        "bad recipient"
    );
    for error in [
        WalletCallError::Unavailable("gone".to_string()),
        WalletCallError::Failed("encoder stopped".to_string()),
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[tokio::test]
async fn a_disabled_host_reports_unavailable_without_starting_a_broker() {
    let mut config = offline_config();
    config.modules.enabled = false;

    let spec = TransactionSpec::Evm {
        to: "0x3535353535353535353535353535353535353535".to_string(),
        value_wei: "1".to_string(),
        data_hex: "0x".to_string(),
        nonce: 0,
        gas_limit: 21_000,
        gas_price_wei: "1".to_string(),
        chain_id: 1,
    };

    assert!(matches!(
        super::sign_transaction_in_module(&config, &spec, &evm_signing_secret()).await,
        Err(WalletCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::ensure_ready(&config).await,
        Err(WalletCallError::Unavailable(_))
    ));
}

// ---------------------------------------------------------------------------
// The attestation guard
// ---------------------------------------------------------------------------

/// The digest check is the one part of `attested_proxy` that does not need a
/// live broker, and it is the part that decides whether a recovery phrase is
/// handed over. The rest of the guard — that an attestation exists at all — is
/// enforced by tinybus and covered by its own tests against a real `dlopen`.
mod attestation_guard {
    use super::super::digest_is_pinned;
    use crate::openhuman::modules::registry;

    fn record() -> &'static crate::openhuman::modules::ModuleRecord {
        registry::find("tinywallet").expect("the tinywallet record is compiled in")
    }

    #[test]
    fn every_pinned_artifact_is_accepted() {
        let record = record();
        assert!(!record.assets.is_empty());
        for asset in record.assets {
            assert!(
                digest_is_pinned(record, asset.sha256),
                "pinned artifact {} was not accepted by its own table",
                asset.archive
            );
        }
    }

    #[test]
    fn the_attested_linux_release_library_is_accepted() {
        assert!(digest_is_pinned(
            record(),
            "2bd70433707c44dbfe6b3cc3b4cc835299fe951fcb375b49c940d8d3fc1d4061"
        ));
    }

    #[test]
    fn a_digest_this_build_did_not_pin_is_refused() {
        // The case that matters: an artifact the host attested but this build
        // never named. Without the check, "the host vouched for something"
        // would be enough to be sent a key.
        assert!(!digest_is_pinned(record(), &"a".repeat(64)));
        assert!(!digest_is_pinned(record(), ""));
    }

    #[test]
    fn a_pinned_digest_is_matched_regardless_of_hex_case() {
        // The release manifest and this table are written by different hands.
        // A case mismatch refusing a legitimate artifact would break signing
        // for every user, and it would look like an attack rather than a typo.
        let record = record();
        let upper = record.assets[0].sha256.to_ascii_uppercase();
        assert_ne!(
            upper, record.assets[0].sha256,
            "fixture must actually differ"
        );
        assert!(digest_is_pinned(record, &upper));
    }
}

/// The confidential request shapes, pinned against the module's own types.
///
/// `call_confidential` is generic over its argument tuple, so nothing checks
/// that the value sent for a method is the type that method takes. That gap is
/// not theoretical: `ExportKey` was first called with a bare `SecretMaterial`
/// where the module expects an `ExportRequest` wrapping one. It compiled, and
/// it would have failed at deserialization on the far side of the bus — the
/// only signal being a runtime error on the one path that exports a key.
///
/// These assert the two shapes are genuinely distinct, so the wrapper cannot be
/// dropped again without a test failing.
mod request_shapes {
    use tinywallet_bus::wire::{ExportRequest, SecretMaterial, SignMessageRequest, SignRequest};

    fn secret() -> SecretMaterial {
        super::evm_signing_secret()
    }

    #[test]
    fn an_export_request_is_not_interchangeable_with_a_bare_secret() {
        let bare = serde_json::to_value(secret()).unwrap();
        assert!(
            serde_json::from_value::<ExportRequest>(bare).is_err(),
            "a bare SecretMaterial must not deserialize as an ExportRequest, or the \
             wrapper could be dropped at a call site without anything noticing"
        );

        let wrapped = serde_json::to_value(ExportRequest { secret: secret() }).unwrap();
        assert!(serde_json::from_value::<ExportRequest>(wrapped).is_ok());
    }

    #[test]
    fn the_wrapped_request_types_do_not_accept_each_other() {
        // `SignRequest` and `SignMessageRequest` both wrap a secret and both
        // take a second field, so a call site that swapped them would still
        // look plausible. `deny_unknown_fields` is what stops that.
        let sign_message = serde_json::to_value(SignMessageRequest {
            secret: secret(),
            message_hex: "00".repeat(32),
            scheme: tinywallet_bus::wire::Scheme::Secp256k1Prehash,
        })
        .unwrap();
        assert!(serde_json::from_value::<SignRequest>(sign_message).is_err());
        assert!(serde_json::from_value::<ExportRequest>(
            serde_json::to_value(SignRequest {
                secret: secret(),
                transaction: tinywallet_bus::wire::TransactionSpec::Evm {
                    to: format!("0x{}", "11".repeat(20)),
                    value_wei: "1".to_string(),
                    data_hex: "0x".to_string(),
                    nonce: 0,
                    gas_limit: 21_000,
                    gas_price_wei: "1".to_string(),
                    chain_id: 1,
                },
            })
            .unwrap()
        )
        .is_err());
    }
}

// ---------------------------------------------------------------------------
// The contract this client compiles against
// ---------------------------------------------------------------------------

/// `registry.rs` is a compiled-in `const` table, and it cannot name a gated
/// crate: its `bus_name` and `object_path` are string literals sitting next to
/// the module's own spelling of them with nothing between the two. A mismatch is
/// therefore not a compile error — it is a `NameHasNoOwner` at first use, in the
/// field, on whichever platform nobody tested. These two tests are what stands
/// in for the compiler.
mod contract {
    use crate::openhuman::modules::registry;

    #[test]
    fn the_registry_entry_matches_the_interface_this_client_calls() {
        let record = registry::find("tinywallet").expect("the tinywallet record is compiled in");
        assert_eq!(record.bus_name, tinywallet_bus::BUS_NAME);
        assert_eq!(record.object_path, tinywallet_bus::OBJECT_PATH);
        assert!(
            record.object_path.starts_with('/') && !record.object_path.contains('.'),
            "an object path with a dot in it is rejected by the loader, not by the compiler"
        );
    }

    #[test]
    fn every_member_this_client_calls_is_one_the_contract_declares() {
        // The direction that matters. A name this host sends that the module
        // does not serve fails at call time with nothing to catch it earlier;
        // the reverse — a member the module serves and this host never calls —
        // is fine, and two of them are exactly that. `BuildUnsigned` and
        // `AttachSignature` are the two-round-trip flow for a backend that
        // cannot be trusted with a key, which does not apply to a module whose
        // artifact this build hashed, so the confidential members are the ones
        // used here.
        use tinywallet_bus::names::methods;

        let called = [
            methods::SIGN_TRANSACTION,
            methods::DERIVE_ACCOUNT,
            methods::SIGN_MESSAGE,
            methods::EXPORT_KEY,
        ];
        for member in called {
            assert!(
                tinywallet_bus::METHODS.contains(&member),
                "{member} is not a member of {}",
                tinywallet_bus::BUS_NAME
            );
            // And every one of them carries a recovery phrase, so every one has
            // to go out over a confidential call rather than a plain one.
            assert!(
                tinywallet_bus::CONFIDENTIAL_METHODS.contains(&member),
                "{member} is called confidentially but not declared confidential"
            );
        }
    }
}
