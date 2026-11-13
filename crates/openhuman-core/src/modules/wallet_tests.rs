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
use crate::config::Config;

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
    assert!(matches!(
        super::generate_mnemonic(&config, 12).await,
        Err(WalletCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::derive_mnemonic_seed(
            &config,
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
                .to_string(),
        )
        .await,
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
    use super::super::{digest_is_pinned, PINNED_WALLET_LIBRARIES};
    use crate::modules::registry;

    fn record() -> &'static crate::modules::ModuleRecord {
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
    fn the_current_attested_linux_release_library_is_accepted() {
        assert!(digest_is_pinned(
            record(),
            "52cf86da7e6b83e3ec79e77fcfea71a7f47628b3e534aa99e33f0c78bd50bb4a"
        ));
    }

    #[test]
    fn libraries_from_retired_releases_are_refused() {
        for digest in [
            // v0.29.0, retired by the v0.30.0 pin
            "ca77309f2d5cdb6d13e56cb7fc44145507a333f55c6a31729979a2782a4ab3ae",
            "06ebfd294c3e815e4fca25df608e2b37f9bfb493b74687562f93b9bd34d52760",
            "d2e663e465c6c245cbf9b37c037ae655db7df0f93a0df9b7fa17a28027ebaa9d",
            "2bd70433707c44dbfe6b3cc3b4cc835299fe951fcb375b49c940d8d3fc1d4061",
            "9459eaaf5b147bdc7f0f33f8fd1828bb50fe359002ebc7fa7efeccacbb0e70bc",
            "1e38209820ffa41857adfc32c322871bee30eec6bfc02dfae46eb7d9f6013def",
            "fd310805c667794548fae37a7f9ceb324185c7a46b6eefc30a7ea82fc428c254",
            "8fee5f20bd58505eb2a291e565d9c83928f4418d5c114413baf55613e0120baf",
            "92d024c6be951824070c80fdcd44661e2f47a812fd82c247a16a810680075b31",
        ] {
            assert!(!digest_is_pinned(record(), digest));
        }
    }

    #[test]
    fn a_release_library_requires_its_archive_to_remain_pinned() {
        let unpinned_record = crate::modules::ModuleRecord {
            assets: &[],
            ..*record()
        };
        let current_library = "52cf86da7e6b83e3ec79e77fcfea71a7f47628b3e534aa99e33f0c78bd50bb4a";
        assert!(digest_is_pinned(record(), current_library));
        assert!(!digest_is_pinned(&unpinned_record, current_library));
    }

    #[test]
    fn every_pinned_platform_archive_has_one_verified_library() {
        for asset in record().assets {
            let libraries: Vec<_> = PINNED_WALLET_LIBRARIES
                .iter()
                .filter(|(archive, _)| archive.eq_ignore_ascii_case(asset.sha256))
                .collect();
            assert_eq!(
                libraries.len(),
                1,
                "refresh the verified library checksum for {}",
                asset.archive
            );
            let (_, library) = libraries[0];
            assert!(digest_is_pinned(record(), library));
            assert!(digest_is_pinned(record(), &library.to_ascii_uppercase()));
        }
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

/// A wallet linked into the host executable (`[modules] source = "linked"`,
/// `openhuman-modules-linked`) is attested by tinybus with the executable's own
/// digest. The attestation these tests feed the gate is the one tinybus really
/// issues: a static-link module answering to the wallet's bus name, admitted
/// through `ModuleHost::attach_linked_with_config` on a private broker, read
/// back over the bus exactly as `attested_proxy` reads it.
mod linked_attestation {
    use std::time::Duration;

    use tinybus::attest::Attestation;
    use tinybus::broker::Broker;
    use tinybus::module::ModuleHost;
    use tinybus::transport::memory::MemoryBus;
    use tinybus::{BusName, Connection};

    use super::super::{admit_attestation, WalletCallError};
    use crate::modules::registry;

    const NOT_PINNED: &str = "the loaded wallet module is not one of the artifacts this build \
                              pinned, so it will not be sent key material";

    mod fixture {
        async fn setup(_: tinybus::Connection) -> tinybus::Result<()> {
            Ok(())
        }

        tinybus_module::module_export_static! {
            setup = setup,
            worker_threads = 1,
            provides = ["ai.tinyhumans.tinywallet.Wallet"],
            methods = [],
            signals = [],
            requires = [],
            optional = [],
            lazy = false,
        }
    }

    fn record() -> &'static crate::modules::ModuleRecord {
        registry::find("tinywallet").expect("the tinywallet record is compiled in")
    }

    /// The digest tinybus attests a linked module with: this test binary's.
    fn this_executable() -> String {
        tinybus::module::sha256_file(std::env::current_exe().expect("test executable"))
            .expect("test executable hashes")
    }

    /// What the broker says about a linked wallet, as a sender sees it.
    async fn linked_wallet_attestation() -> Attestation {
        let record = record();
        let transport = MemoryBus::new();
        let broker = Broker::new();
        broker.spawn(transport.clone());
        let modules = ModuleHost::new(broker);
        let sender = Connection::connect(transport.connect().await.unwrap())
            .await
            .unwrap();
        modules
            .attach_linked_with_config(fixture::linked_module().unwrap(), serde_json::json!({}))
            .expect("the linked fixture is admitted");
        let name = BusName::new(record.bus_name).unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(attestation) = sender.attestation(name.clone()).await.unwrap() {
                return attestation;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the linked wallet was never attested"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn a_wallet_linked_into_this_executable_is_sent_key_material() {
        let attestation = linked_wallet_attestation().await;
        let host_code = this_executable();
        assert_eq!(attestation.name.as_str(), record().bus_name);
        assert_eq!(
            attestation.sha256, host_code,
            "tinybus attests linked code with the executable's digest"
        );
        assert_eq!(attestation.library_sha256, None);

        assert_eq!(
            admit_attestation(record(), &attestation, Some(&host_code)),
            Ok(())
        );
    }

    #[tokio::test]
    async fn the_executable_digest_is_refused_when_this_binary_links_no_wallet() {
        // A build that registered no linked tinywallet never accepts host code
        // for it: the release pins are the only artifacts it can be sent to.
        let attestation = linked_wallet_attestation().await;
        assert_eq!(
            admit_attestation(record(), &attestation, None),
            Err(WalletCallError::Failed(NOT_PINNED.to_string()))
        );
    }

    #[tokio::test]
    async fn a_digest_neither_pinned_nor_this_executable_is_refused() {
        let attestation = Attestation {
            sha256: "a".repeat(64),
            ..linked_wallet_attestation().await
        };
        assert_eq!(
            admit_attestation(record(), &attestation, Some(&this_executable())),
            Err(WalletCallError::Failed(NOT_PINNED.to_string()))
        );
        let empty = Attestation {
            sha256: String::new(),
            ..attestation
        };
        assert_eq!(
            admit_attestation(record(), &empty, Some(&this_executable())),
            Err(WalletCallError::Failed(NOT_PINNED.to_string()))
        );
    }

    #[tokio::test]
    async fn host_code_answering_to_another_name_is_refused() {
        let attestation = Attestation {
            name: BusName::new("ai.tinyhumans.tinydocs.Documents").unwrap(),
            ..linked_wallet_attestation().await
        };
        assert!(matches!(
            admit_attestation(record(), &attestation, Some(&this_executable())),
            Err(WalletCallError::Failed(message)) if message.contains("rather than")
        ));
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
    use crate::modules::registry;

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
