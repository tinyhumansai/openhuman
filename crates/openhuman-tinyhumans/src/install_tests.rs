//! `install` and the builder share `wiring`: these pin what each option
//! resolves to and that `install` applies it to the process globals.

use super::*;
#[cfg(feature = "jev")]
use openhuman_embed::__host::agent::tinyagents::discovery::installed_tool_ranker;
use openhuman_embed::seams::DomainGroup;

#[test]
fn default_wiring_carries_hosted_controllers_and_the_jev_ranker() {
    let _registry = crate::registry_test_lock();

    let wired = wiring(&InstallOptions::default()).expect("wiring");
    assert_eq!(
        wired.controllers.as_ref().map(|ext| ext.group),
        Some(DomainGroup::Hosted)
    );
    #[cfg(feature = "jev")]
    assert!(wired.ranker.is_some());
    #[cfg(not(feature = "jev"))]
    assert!(wired.ranker.is_none());
    assert!(is_installed());
    let shown = format!("{wired:?}");
    assert!(shown.contains("Hosted"), "{shown}");
}

#[test]
fn options_turn_controllers_and_ranker_off() {
    let _registry = crate::registry_test_lock();

    let wired = wiring(
        &InstallOptions::default()
            .hosted_controllers(false)
            .tool_ranker(false),
    )
    .expect("wiring");
    assert!(wired.controllers.is_none());
    assert!(wired.ranker.is_none());
}

#[test]
fn wiring_reuses_the_process_transport() {
    let _registry = crate::registry_test_lock();

    // A product-identity change drops the cached transport; hold its lock.
    let _guard = crate::backend::product::product_identity_test_lock();
    let first = wiring(&InstallOptions::default()).expect("first");
    let second = wiring(&InstallOptions::default()).expect("second");
    assert!(Arc::ptr_eq(&first.transport, &second.transport));
}

#[test]
fn install_registers_hosted_controllers_and_installs_the_ranker() {
    let _registry = crate::registry_test_lock();

    // A product-identity change drops the cached transport; hold its lock.
    let _guard = crate::backend::product::product_identity_test_lock();
    let transport = install(InstallOptions::default()).expect("install");
    assert!(is_installed());
    assert!(
        openhuman_embed::schema_for_rpc_method("openhuman.billing_get_summary").is_some(),
        "hosted controllers must be in the core registry"
    );
    #[cfg(feature = "jev")]
    assert!(installed_tool_ranker().is_some());
    // The same transport the builder path binds.
    let wired = wiring(&InstallOptions::default()).expect("wiring");
    assert!(Arc::ptr_eq(&transport, &wired.transport));
}
