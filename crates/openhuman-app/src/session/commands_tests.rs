#[test]
fn session_commands_are_allowed_by_the_main_window_capability() {
    let capability = include_str!("../../permissions/allow-core-process.toml");

    for command in [
        "auth_login_with_token",
        "auth_store_session",
        "auth_logout",
        "auth_state",
        "auth_current_user",
    ] {
        assert!(
            capability.contains(&format!("\"{command}\"")),
            "session command {command} is missing from allow-core-process.toml"
        );
    }
}
