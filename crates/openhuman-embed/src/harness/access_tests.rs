use super::*;
use openhuman_core::config::Config;

/// The regression this whole type exists for.
///
/// `full()` must set the tier **and** the origin. Setting only the tier
/// produces an agent whose acting tools all refuse while the transcript still
/// reads plausibly, which is the expensive failure to diagnose.
#[test]
fn full_sets_both_the_tier_and_the_origin() {
    let access = Access::full();

    let mut config = Config::default();
    access.apply(&mut config);
    assert_eq!(config.autonomy.level, AutonomyLevel::Full);

    let origin = access.turn_origin().expect(
        "Access::full must label its turns; without an origin the approval gate \
         fail-closes and every external-effect tool refuses regardless of tier",
    );
    assert!(matches!(
        origin,
        AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::Workflow {
                require_approval: false
            },
            ..
        }
    ));
    assert!(!access.approval_gate_enabled());
}

#[test]
fn readonly_and_supervised_leave_the_origin_to_the_core() {
    // Labelling these as trusted automation would hand them an allowance the
    // tier is meant to withhold.
    for access in [Access::readonly(), Access::supervised()] {
        assert!(access.turn_origin().is_none());
        assert!(access.approval_gate_enabled());
    }
}

#[test]
fn each_preset_applies_its_tier() {
    for (access, expected) in [
        (Access::readonly(), AutonomyLevel::ReadOnly),
        (Access::supervised(), AutonomyLevel::Supervised),
        (Access::full(), AutonomyLevel::Full),
    ] {
        let mut config = Config::default();
        access.apply(&mut config);
        assert_eq!(config.autonomy.level, expected);
    }
}

#[test]
fn the_default_is_supervised() {
    let mut config = Config::default();
    Access::default().apply(&mut config);
    assert_eq!(config.autonomy.level, AutonomyLevel::Supervised);
}

#[test]
fn tool_install_is_opt_in_even_at_full() {
    // Installing software on the host reaches outside the action directory that
    // otherwise bounds the blast radius, so no tier implies it.
    let mut config = Config::default();
    Access::full().apply(&mut config);
    assert!(!config.autonomy.allow_tool_install);

    let mut config = Config::default();
    Access::full().allow_tool_install(true).apply(&mut config);
    assert!(config.autonomy.allow_tool_install);
}

#[test]
fn full_does_not_set_auto_approve_all() {
    // `auto_approve_all` is a blanket bypass that would also cover call sites
    // this harness never vouched for. The origin is the scoped instrument.
    let mut config = Config::default();
    Access::full().apply(&mut config);
    assert!(!config.autonomy.auto_approve_all);
}

#[test]
fn trusted_roots_accumulate_and_keep_their_access_level() {
    let mut config = Config::default();
    let before = config.autonomy.trusted_roots.len();

    Access::supervised()
        .trust("/srv/data", TrustedAccess::Read)
        .trust("/srv/out", TrustedAccess::ReadWrite)
        .apply(&mut config);

    let added = &config.autonomy.trusted_roots[before..];
    assert_eq!(added.len(), 2);
    assert_eq!(added[0].path, "/srv/data");
    assert_eq!(added[0].access, TrustedAccess::Read);
    assert_eq!(added[1].path, "/srv/out");
    assert_eq!(added[1].access, TrustedAccess::ReadWrite);
}

/// Regression for a narrower per-agent `Access` leaking a wider caller's
/// trusted roots: `Runtime::agent` starts each agent's config from the
/// runtime's base config, which already carries the runtime default
/// `Access::apply`, then applies the agent's own `Access` on top. If `apply`
/// extended rather than replaced `trusted_roots`, a supposedly `readonly()`
/// agent would keep every root the runtime-wide default trusted.
#[test]
fn apply_replaces_trusted_roots_rather_than_extending_them() {
    let mut config = Config::default();
    Access::full()
        .trust("/srv/wide", TrustedAccess::ReadWrite)
        .apply(&mut config);
    assert_eq!(config.autonomy.trusted_roots.len(), 1);

    // A second, narrower `Access` applied to the same config (as happens when
    // an agent overrides the runtime's default) must not keep the first
    // access's trusted roots around.
    Access::readonly().apply(&mut config);
    assert!(
        config.autonomy.trusted_roots.is_empty(),
        "a readonly access must not inherit an earlier access's trusted roots, \
         got {:?}",
        config.autonomy.trusted_roots
    );
}

#[test]
fn an_explicit_origin_overrides_the_preset() {
    let access = Access::full().origin(AgentTurnOrigin::TrustedAutomation {
        job_id: "nightly".into(),
        source: TrustedAutomationSource::Cron,
    });
    assert!(matches!(
        access.turn_origin(),
        Some(AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::Cron,
            ..
        })
    ));
}

#[test]
fn approval_lists_are_inherited_until_the_agent_sets_them() {
    let mut config = Config::default();
    config.autonomy.auto_approve = vec!["boot_tool".into()];
    config.autonomy.auto_approve_all = true;

    Access::supervised().apply(&mut config);
    assert_eq!(config.autonomy.auto_approve, ["boot_tool"]);
    assert!(config.autonomy.auto_approve_all);

    Access::supervised()
        .auto_approve(["shell", "file_write"])
        .auto_approve_all(false)
        .apply(&mut config);
    assert_eq!(config.autonomy.auto_approve, ["shell", "file_write"]);
    assert!(!config.autonomy.auto_approve_all);
}

#[test]
fn the_approval_gate_switch_is_independent_of_the_tier() {
    let access = Access::supervised().approval_gate(false);
    assert!(!access.approval_gate_enabled());
    let mut config = Config::default();
    access.apply(&mut config);
    assert_eq!(config.autonomy.level, AutonomyLevel::Supervised);
    assert!(access.turn_origin().is_none());

    assert!(Access::full().approval_gate(true).approval_gate_enabled());
}

/// The core ships the policy off, which makes every tier inert. A scoped-down
/// access level must switch it on, or `readonly()` would still let the agent
/// write files and run commands.
#[test]
fn restrictive_tiers_switch_the_policy_on() {
    for access in [Access::readonly(), Access::supervised()] {
        let mut config = openhuman_core::config::Config::default();
        config.autonomy.enabled = false;
        access.apply(&mut config);
        assert!(config.autonomy.enabled, "{access:?} must enable the policy");
    }
}

/// `full()` leaves the switch where the base config put it.
#[test]
fn full_access_keeps_the_configured_policy_switch() {
    for enabled in [false, true] {
        let mut config = openhuman_core::config::Config::default();
        config.autonomy.enabled = enabled;
        Access::full().apply(&mut config);
        assert_eq!(config.autonomy.enabled, enabled);
    }
}
