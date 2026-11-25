use super::*;

#[test]
fn off_value_accepts_off_spellings() {
    for v in ["off", "OFF", " none ", "0", "false", "Disabled"] {
        assert!(off_value(Some(v)), "{v:?} should mean the sandbox is off");
    }
}

#[test]
fn off_value_keeps_the_sandbox_on_otherwise() {
    for v in [
        None,
        Some(""),
        Some("on"),
        Some("1"),
        Some("true"),
        Some("local"),
    ] {
        assert!(!off_value(v), "{v:?} must leave the sandbox on");
    }
}

#[test]
fn the_component_switch_reads_the_context_the_call_runs_under() {
    assert!(!component_off(&DispatchCtx::ambient()));
    let mut set = crate::core::runtime::ComponentSet::all();
    set.sandbox = false;
    let off =
        DispatchCtx::new(crate::core::runtime::context::CoreContext::for_test_with_components(set));
    assert!(component_off(&off));
    let on = DispatchCtx::new(
        crate::core::runtime::context::CoreContext::for_test_with_components(
            crate::core::runtime::ComponentSet::all(),
        ),
    );
    assert!(!component_off(&on));
}

#[test]
fn the_saved_configuration_turns_the_sandbox_off_by_flag_or_component() {
    let mut config = Config::default();
    assert!(!config_off(&config));
    config.sandbox.enabled = Some(false);
    assert!(config_off(&config));
    config.sandbox.enabled = None;
    config.components.sandbox = false;
    assert!(config_off(&config));
}

#[test]
fn the_setting_is_read_without_asking_for_the_deployment() {
    // Whether the deployment honours it is the module's decision, not this reader's.
    let config = Config::default();
    let ctx = DispatchCtx::unbound();
    let mut disabled = config.clone();
    disabled.sandbox.enabled = Some(false);
    assert!(sandbox_off(&ctx, &disabled));
}
