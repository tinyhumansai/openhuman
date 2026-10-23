use super::*;

#[test]
fn command_hooks_always_use_the_sandboxed_host_spawner() {
    let environment = environment();

    assert!(
        environment.command_spawner.is_some(),
        "OpenHuman command hooks must not fall back to the hook engine's local shell"
    );
}
