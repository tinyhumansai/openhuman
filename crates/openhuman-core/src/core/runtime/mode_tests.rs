use super::*;

#[test]
fn parses_both_modes_and_rejects_anything_else() {
    assert_eq!("saas".parse::<Mode>(), Ok(Mode::Saas));
    assert_eq!(" SaaS ".parse::<Mode>(), Ok(Mode::Saas));
    assert_eq!("single-user".parse::<Mode>(), Ok(Mode::SingleUser));
    assert_eq!("single_user".parse::<Mode>(), Ok(Mode::SingleUser));
    assert!("multi".parse::<Mode>().is_err());
    assert!("".parse::<Mode>().is_err());
}

#[test]
fn tags_round_trip() {
    for mode in [Mode::SingleUser, Mode::Saas] {
        assert_eq!(mode.tag().parse::<Mode>(), Ok(mode));
    }
}

#[test]
fn an_unlocked_process_is_single_user() {
    assert_eq!(Mode::default(), Mode::SingleUser);
}

// The process slot is shared by every test in the binary, so the locking rule
// is exercised on a local slot.
#[test]
fn relocking_the_same_mode_is_fine_but_switching_fails() {
    let slot = OnceLock::new();
    assert!(lock_in(&slot, Mode::Saas).is_ok());
    assert!(lock_in(&slot, Mode::Saas).is_ok());
    let err = lock_in(&slot, Mode::SingleUser).unwrap_err();
    assert!(err.contains("saas"), "{err}");
    assert_eq!(slot.get(), Some(&Mode::Saas));
}

#[test]
fn saas_requests_are_spotted_before_parsing() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(requested_in(&args(&["run", "--mode", "saas"]), None));
    assert!(requested_in(&args(&["run", "--mode=saas"]), None));
    assert!(requested_in(&args(&["run"]), Some("saas")));
    assert!(!requested_in(
        &args(&["run", "--mode", "single-user"]),
        None
    ));
    assert!(!requested_in(&args(&["run"]), None));
    assert!(!requested_in(&args(&["run", "saas"]), Some("")));
}

#[test]
fn a_saas_core_needs_the_saas_boot() {
    use crate::core::types::HostKind;
    // This test process is never locked to SaaS.
    assert!(admit_core(HostKind::Cli, false).is_ok());
    let refused = admit_core(HostKind::Saas, false).unwrap_err();
    assert!(refused.contains("saas::build"), "{refused}");
}
