use super::*;

#[test]
fn first_run_prompt_requests_initial_population() {
    let p = build_prompt("user wants to learn rust", true);
    assert!(p.contains("EMPTY"));
    assert!(p.contains("first run"));
    assert!(p.contains("user wants to learn rust"));
}

#[test]
fn maintenance_prompt_requests_minimal_changes() {
    let p = build_prompt("user finished onboarding", false);
    assert!(p.contains("MINIMAL"));
    assert!(!p.contains("first run"));
    assert!(p.contains("user finished onboarding"));
}
