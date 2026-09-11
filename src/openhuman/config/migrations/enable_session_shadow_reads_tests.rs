use super::*;

#[test]
fn flips_a_persisted_opt_out_on() {
    let mut config = Config::default();
    config.agent.session_shadow_reads = false;

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.shadow_reads_enabled, 1);
    assert!(config.agent.session_shadow_reads);
}

#[test]
fn leaves_an_already_enabled_workspace_untouched() {
    let mut config = Config::default();
    config.agent.session_shadow_reads = true;

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.shadow_reads_enabled, 0);
    assert!(config.agent.session_shadow_reads);
}

#[test]
fn is_idempotent_across_repeated_runs() {
    let mut config = Config::default();
    config.agent.session_shadow_reads = false;

    run(&mut config).expect("first run should succeed");
    let second = run(&mut config).expect("second run should succeed");

    assert_eq!(second.shadow_reads_enabled, 0);
    assert!(config.agent.session_shadow_reads);
}
