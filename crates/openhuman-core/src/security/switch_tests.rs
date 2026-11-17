use super::*;
use crate::config::{Config, SecurityModeConfig};

fn off_config() -> Config {
    Config {
        security: SecurityModeConfig::off(),
        ..Config::default()
    }
}

#[cfg(not(feature = "security-module"))]
#[test]
fn a_build_without_the_module_is_disabled_whatever_the_config_says() {
    assert_eq!(disabled(), Some(DisabledReason::FeatureAbsent));
    assert_eq!(
        disabled_for(Some(&Config::default())),
        Some(DisabledReason::FeatureAbsent)
    );
    assert_eq!(disabled_for(None), Some(DisabledReason::FeatureAbsent));
}

#[cfg(feature = "security-module")]
#[test]
fn an_enforcing_config_leaves_the_checks_on() {
    assert_eq!(disabled(), None);
    assert_eq!(disabled_for(None), None);
    assert_eq!(disabled_for(Some(&Config::default())), None);
}

#[cfg(feature = "security-module")]
#[test]
fn a_config_that_says_off_disables_the_checks_for_its_own_caller() {
    assert_eq!(
        disabled_for(Some(&off_config())),
        Some(DisabledReason::ConfiguredOff)
    );
    // The config alone does not reach sites that hold none.
    assert_eq!(disabled(), None);
}

#[cfg(feature = "security-module")]
#[test]
fn booting_with_security_off_disables_the_checks_process_wide() {
    apply_boot_config(&off_config());
    assert_eq!(disabled(), Some(DisabledReason::ConfiguredOff));
    assert_eq!(
        disabled_for(Some(&Config::default())),
        Some(DisabledReason::ConfiguredOff)
    );
    apply_boot_config(&Config::default());
    assert_eq!(disabled(), None);
}

#[cfg(feature = "security-module")]
#[test]
fn the_test_guard_restores_the_previous_mode_on_drop() {
    {
        let _off = off_for_this_thread();
        assert_eq!(disabled(), Some(DisabledReason::ConfiguredOff));
    }
    assert_eq!(disabled(), None);
}

#[cfg(feature = "security-module")]
#[test]
fn the_test_guard_does_not_reach_another_thread() {
    let _off = off_for_this_thread();
    let elsewhere = std::thread::spawn(disabled).join().unwrap();
    assert_eq!(elsewhere, None);
}

#[test]
fn reasons_are_described_for_the_boot_line() {
    assert_eq!(
        DisabledReason::FeatureAbsent.describe(),
        "built without the security-module feature"
    );
    assert_eq!(
        DisabledReason::ConfiguredOff.describe(),
        "[security] mode = \"off\""
    );
}

#[test]
fn a_config_alone_describes_its_mode_whatever_the_process_booted() {
    #[cfg(feature = "security-module")]
    {
        let _off = off_for_this_thread();
        assert_eq!(disabled_by_config(&Config::default()), None);
        assert_eq!(
            disabled_by_config(&off_config()),
            Some(DisabledReason::ConfiguredOff)
        );
    }
    #[cfg(not(feature = "security-module"))]
    assert_eq!(
        disabled_by_config(&off_config()),
        Some(DisabledReason::FeatureAbsent)
    );
}
