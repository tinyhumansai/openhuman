//! Register the linked module factories for a local test run.
//!
//! Only meaningful with `--features linked-modules` (the whole file is empty
//! without it). CI never enables the feature or sets
//! `OPENHUMAN_MODULE_SOURCE=linked`, so it keeps loading released modules.
//!
//! Include with `#[path = "support/linked_modules.rs"] mod linked_modules;`.

#![allow(dead_code)]

/// Register every linked module with the core. Idempotent.
#[cfg(feature = "linked-modules")]
pub fn install() {
    openhuman_modules_linked::install();
}

/// [`install`] when `OPENHUMAN_MODULE_SOURCE=linked` is set, for suites that
/// should follow the environment like the binary does.
#[cfg(feature = "linked-modules")]
pub fn install_if_requested() {
    if std::env::var("OPENHUMAN_MODULE_SOURCE")
        .is_ok_and(|value| value.eq_ignore_ascii_case("linked"))
    {
        install();
    }
}
