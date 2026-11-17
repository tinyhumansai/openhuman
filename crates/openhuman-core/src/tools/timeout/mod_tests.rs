use super::*;

/// The timeout settings are process-global; tests that push values serialize here.
static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn config_push_reaches_module_and_harness_settings() {
    let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Skip when an operator override would legitimately win.
    if env_override_active() {
        return;
    }
    let effective = set_tool_timeout_secs(321);
    assert_eq!(effective, 321);
    assert_eq!(tool_execution_timeout_secs(), 321);
    assert_eq!(
        harness_settings().inherited_timeout().map(|d| d.as_secs()),
        Some(321)
    );
    set_tool_timeout_secs(module::DEFAULT_TIMEOUT_SECS);
}

#[test]
fn out_of_range_config_push_falls_back_to_default() {
    let _guard = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if env_override_active() {
        return;
    }
    assert_eq!(set_tool_timeout_secs(0), module::DEFAULT_TIMEOUT_SECS);
    assert_eq!(set_tool_timeout_secs(99_999), module::DEFAULT_TIMEOUT_SECS);
}

#[test]
fn env_var_name_is_the_documented_wire_name() {
    assert_eq!(ENV_VAR, "OPENHUMAN_TOOL_TIMEOUT_SECS");
}

#[cfg(unix)]
#[tokio::test]
async fn adapter_wrapped_waiter_reaps_a_timed_out_group() {
    ensure_configured();
    let mut cmd = tokio::process::Command::new("sh");
    cmd.args(["-c", "sleep 30 & wait"]);
    let started = std::time::Instant::now();
    let out = module::output_or_kill(&mut cmd, std::time::Duration::from_millis(200)).await;
    assert!(out.is_err(), "deadline should elapse");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}
