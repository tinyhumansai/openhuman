use super::*;
use tinysecurity_bus::CommandClass;

// -- LLM escalate-only category (Phase G) -------------------------

#[test]
fn parse_declared_class_maps_known_and_rejects_unknown() {
    assert_eq!(
        SecurityPolicy::parse_declared_class("destructive"),
        Some(CommandClass::Destructive)
    );
    assert_eq!(
        SecurityPolicy::parse_declared_class("  WRITE "),
        Some(CommandClass::Write)
    );
    assert_eq!(
        SecurityPolicy::parse_declared_class("network"),
        Some(CommandClass::Network)
    );
    assert_eq!(
        SecurityPolicy::parse_declared_class("install"),
        Some(CommandClass::Install)
    );
    assert_eq!(SecurityPolicy::parse_declared_class("bogus"), None);
    assert_eq!(SecurityPolicy::parse_declared_class(""), None);
    // Escalate-only contract: max() raises but never lowers.
    assert_eq!(
        CommandClass::Write.max(CommandClass::Destructive),
        CommandClass::Destructive
    );
    assert_eq!(
        CommandClass::Destructive.max(CommandClass::Read),
        CommandClass::Destructive
    );
}

// -- is_path_allowed ----------------------------------------------

// -- is_path_allowed — symlink safety (#1927) ---------------------

// -- auto_approve defaults ----------------------------------------
