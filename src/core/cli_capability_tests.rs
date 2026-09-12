//! Tests for the CLI's memory-capability gate.
//!
//! Everything here drives the PURE helpers (`capability_verdict`,
//! `capability_unavailable_message`) plus a directly-resolved binding, rather
//! than `run_from_cli_args`. Reaching a narrowed capability set end-to-end
//! would need `driver = "null"` written into a real `config.toml` under a
//! process-global `OPENHUMAN_WORKSPACE`, i.e. env mutation plus disk writes,
//! and `run_from_cli_args` also loads dotenv and prints the banner. The helper
//! assertions are stronger, not weaker: they can actually reach the null-driver
//! state deterministically.

use super::*;
use crate::openhuman::config::schema::MemorySubsystemConfig;
use crate::openhuman::memory::binding;

fn binding_for(name: &str, cfg: MemorySubsystemConfig) -> std::sync::Arc<binding::MemoryBinding> {
    let dir = std::env::temp_dir().join(format!("oh-cli-cap-{name}"));
    binding::for_workspace(&dir, &cfg).expect("binding resolves")
}

fn null_cfg() -> MemorySubsystemConfig {
    MemorySubsystemConfig {
        driver: "null".into(),
        ..Default::default()
    }
}

#[test]
fn verdict_is_ok_for_ungated_surface() {
    assert!(capability_verdict(
        "null",
        Capabilities::mandatory(),
        None,
        "openhuman memory docs"
    )
    .is_ok());
}

#[test]
fn verdict_names_the_driver_and_the_capability() {
    let err = capability_verdict(
        "null",
        Capabilities::mandatory(),
        Some(Capability::Tree),
        "openhuman memory_tree list_chunks",
    )
    .expect_err("a mandatory-only driver does not advertise `tree`");
    let msg = err.to_string();
    assert!(msg.contains("null"), "{msg}");
    assert!(msg.contains("tree"), "{msg}");
    assert!(msg.contains("openhuman memory_tree list_chunks"), "{msg}");
}

#[test]
fn verdict_error_does_not_read_like_a_typo() {
    let err = capability_verdict(
        "null",
        Capabilities::mandatory(),
        Some(Capability::Tree),
        "openhuman memory_tree list_chunks",
    )
    .expect_err("gated");
    let msg = err.to_string();
    assert!(!msg.contains("unknown namespace"), "{msg}");
    assert!(!msg.contains("unknown function"), "{msg}");
    assert!(!msg.contains("unknown method"), "{msg}");
}

#[test]
fn verdict_is_ok_when_the_driver_advertises_the_family() {
    assert!(capability_verdict(
        "tinycortex",
        Capabilities::all(),
        Some(Capability::Tree),
        "openhuman memory_tree list_chunks",
    )
    .is_ok());
}

/// The message carries a driver id and a capability constant and nothing else —
/// never the configured endpoint or credential reference.
#[test]
fn message_never_contains_a_credential_or_endpoint() {
    let msg = capability_unavailable_message(
        "supermemory",
        Capability::Tree,
        "openhuman memory_tree list_chunks",
    );
    assert!(!msg.contains("keychain:"), "{msg}");
    assert!(!msg.contains("api.supermemory.ai"), "{msg}");
    assert!(msg.starts_with(CAPABILITY_UNAVAILABLE_PREFIX), "{msg}");
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn bound_driver_probe_reports_the_default_module_driver() {
    // Was asserting `capabilities() == Capabilities::all()`. That encoded #5598
    // as expected: the then-pinned v1.0.1 artifact served thirteen of the
    // contract's eighteen families, so claiming the full contract made the
    // other five answer `UnknownMethod` instead of reporting themselves absent.
    //
    // The boundary has since moved twice more: v1.2.0 added bus members for
    // `chunks`, `people`, `profile` and `retrieval` (thirteen -> seventeen),
    // and the Episodic accessor landing with the archivist migration closed the
    // last host gap (seventeen -> eighteen). Full advertisement is the honest
    // set now — every family has both a bus member in the pinned artifact and a
    // host accessor. What still guards drift is the accessor rule itself
    // (`capabilities_for` can only name families the provider implements) plus
    // the pin-drift test on every registry bump.
    let cfg = MemorySubsystemConfig::default();
    let binding = binding_for("default", cfg.clone());
    assert_eq!(
        binding.driver_id(),
        crate::openhuman::memory::binding::MODULE_ID
    );
    let advertised = binding.capabilities();
    assert!(advertised.contains_all(Capabilities::mandatory()));
    assert!(advertised.contains(Capability::Tree));
    for capability in [
        Capability::Chunks,
        Capability::People,
        Capability::Profile,
        Capability::Retrieval,
        Capability::Episodic,
    ] {
        assert!(
            advertised.contains(capability),
            "the pinned artifact serves {capability:?} and the host has an accessor for it — \
             hiding it is an under-claim"
        );
    }
    assert!(Capabilities::all().contains_all(advertised));
    assert_eq!(advertised, Capabilities::all());
}

/// The negative control that makes the assertions above mean something.
#[test]
fn null_driver_probe_advertises_only_the_mandatory_families() {
    let binding = binding_for("null", null_cfg());
    assert_eq!(binding.driver_id(), "null");
    assert!(!binding.capabilities().contains(Capability::Tree));
    assert!(!binding.capabilities().contains(Capability::Ingest));
}

/// A genuine typo must never become a capability error: the gate is skipped
/// entirely when no such controller is registered.
#[test]
fn ensure_capability_blocking_is_a_noop_for_an_unknown_controller() {
    assert_eq!(
        crate::core::all::capability_for_parts("does_not_exist", "nope"),
        None
    );
    assert!(
        ensure_capability_blocking(None, "openhuman does_not_exist nope").is_ok(),
        "an unregistered controller must not be reported as a capability fact"
    );
}
