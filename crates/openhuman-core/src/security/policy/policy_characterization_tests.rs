//! Phase-0 characterization of the host policy that the tinysecurity bus module
//! must reproduce (issue #7328, plan Task 1). These tables pin today's behaviour
//! in one place: command classes, the autonomy gate matrix, the allowlist under
//! each tier, the always-forbidden floor, trusted roots and the hourly limiter.
//! They bless what exists; desired changes are written as separate tests whose
//! names start with `desired_` and which are expected to fail until fixed.

use super::*;
use tinysecurity_bus::CommandClass;

fn policy(autonomy: AutonomyLevel) -> SecurityPolicy {
    SecurityPolicy {
        enabled: true,
        autonomy,
        ..SecurityPolicy::default()
    }
}

#[test]
fn declared_class_parses_and_is_escalate_only_input() {
    assert_eq!(
        SecurityPolicy::parse_declared_class(" Destructive "),
        Some(CommandClass::Destructive)
    );
    assert_eq!(
        SecurityPolicy::parse_declared_class("read"),
        Some(CommandClass::Read)
    );
    assert_eq!(SecurityPolicy::parse_declared_class("bogus"), None);
    assert_eq!(SecurityPolicy::parse_declared_class(""), None);
}

#[test]
fn command_class_uses_the_tinysecurity_contract_type() {
    let parsed: tinysecurity_bus::CommandClass =
        SecurityPolicy::parse_declared_class("read").expect("read is a known class");

    assert_eq!(parsed, tinysecurity_bus::CommandClass::Read);
}
