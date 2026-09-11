use super::*;
use crate::openhuman::config::Config;

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
