use super::*;

#[test]
fn net_capability_names_is_outbound_client_only() {
    // `Jail.allow_net` is documented as "Allow outbound network" in
    // `jail.rs`, so the granted capability set must be strictly
    // outbound-client.
    assert!(NET_CAPABILITY_NAMES.contains(&"internetClient"));
    // The following would over-grant inbound bind() rights and are
    // intentionally NOT exposed via the coarse `allow_net` switch.
    // A future caller that needs them should add a separate `Jail`
    // flag (e.g. `allow_private_lan_server`) and gate them on it.
    assert!(!NET_CAPABILITY_NAMES.contains(&"privateNetworkClientServer"));
    assert!(!NET_CAPABILITY_NAMES.contains(&"internetClientServer"));
}

#[test]
fn derive_capability_resolves_well_known_internet_client() {
    // `internetClient` is one of the Windows manifest-defined
    // capabilities present on every supported Windows version
    // (10+). DeriveCapabilitySidsFromName must succeed and return
    // at least one non-null capability SID.
    let deriv = unsafe { derive_capability("internetClient") }
        .expect("internetClient must resolve on any supported Windows");
    assert!(
        !deriv.capability_sids.is_empty(),
        "expected at least one capability SID for internetClient"
    );
    for &sid in &deriv.capability_sids {
        assert!(!sid.is_null(), "capability SID must not be null");
    }
    // Dropping `deriv` here exercises the Drop impl that LocalFrees
    // every SID + array — a no-op for the test runner unless it
    // double-frees, which would crash the process.
}

#[test]
fn sanitize_profile_name_keeps_alnum_and_dot() {
    let s = sanitize_profile_name("agent.delegate-7");
    assert!(s.starts_with("openhuman."));
    // hyphen mapped to underscore
    assert!(s.contains("agent.delegate_7"));
    assert!(s.len() <= 70); // "openhuman." (10) + label (≤60)
}

#[test]
fn sanitize_profile_name_truncates_long_labels() {
    let long: String = "a".repeat(200);
    let s = sanitize_profile_name(&long);
    // Per the in-function comment: truncate to 60 then prefix.
    assert!(s.len() <= 70);
    assert!(s.starts_with("openhuman."));
}

/// PR #4723 review — `AppContainerBackend::is_available()` must
/// stay `false` until the spawn path can return a
/// `std::process::Child`. Reporting available strands a
/// successfully-spawned `cmd.exe` process because `spawn_in_container`
/// currently answers `Err(Unsupported)` after `CreateProcessW`, and
/// callers (e.g. `execute_local_jail`) drop the child on the floor.
/// Flip back to `true` in the same commit that lands the
/// `OwnedHandle -> Child` bridge.
#[test]
fn appcontainer_backend_reports_unavailable_until_child_bridge_lands() {
    assert!(
        !AppContainerBackend::new().is_available(),
        "AppContainer must report unavailable while its spawn path \
         cannot yield a waitable std::process::Child — see #4705 / \
         PR #4723 for the orphan-spawn hazard"
    );
}
