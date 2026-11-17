use super::*;

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

#[test]
fn serve_args_without_flags_leave_every_override_unset() {
    let boot = crate::core::server_launcher::HostBoot::new(1u8);
    let (request, verbose) = parse_serve_args(&[], None, Some(boot.clone()))
        .unwrap()
        .expect("a request");
    assert!(!verbose);
    // Nothing explicit: the host's builder keeps its own listener and services.
    assert_eq!(request.host, None);
    assert_eq!(request.port, None);
    assert!(request.socketio_enabled);
    assert!(!request.headless_api);
    assert_eq!(request.mode, crate::core::runtime::Mode::SingleUser);
    assert_eq!(request.host_boot, Some(boot), "the builder rides along");
}

#[test]
fn serve_args_flags_are_explicit_overrides_on_top_of_the_host_boot() {
    let boot = crate::core::server_launcher::HostBoot::new(1u8);
    let (request, verbose) = parse_serve_args(
        &strings(&[
            "--port",
            "7801",
            "--host",
            "0.0.0.0",
            "--jsonrpc-only",
            "-v",
        ]),
        None,
        Some(boot.clone()),
    )
    .unwrap()
    .unwrap();
    assert!(verbose);
    assert_eq!(request.port, Some(7801));
    assert_eq!(request.host.as_deref(), Some("0.0.0.0"));
    assert!(!request.socketio_enabled);
    assert_eq!(request.host_boot, Some(boot));

    let (headless, _) = parse_serve_args(&strings(&["--headless-api"]), None, None)
        .unwrap()
        .unwrap();
    assert!(headless.headless_api && !headless.socketio_enabled);
    assert!(headless.host_boot.is_none());
}

#[test]
fn serve_args_mode_precedence_is_unchanged_by_the_host_boot() {
    let boot = crate::core::server_launcher::HostBoot::new(1u8);
    // The environment can raise a run to SaaS but a flag cannot lower it.
    let err = parse_serve_args(&[], Some("saas"), Some(boot.clone())).unwrap_err();
    assert!(err.to_string().contains("--saas-config"), "{err}");
    let (request, _) = parse_serve_args(
        &strings(&["--mode", "saas", "--saas-config", "/etc/oh.toml"]),
        None,
        Some(boot),
    )
    .unwrap()
    .unwrap();
    assert_eq!(request.mode, crate::core::runtime::Mode::Saas);
}

#[test]
fn serve_args_reject_unknown_flags_and_print_help_without_a_request() {
    assert!(parse_serve_args(&strings(&["--bogus"]), None, None).is_err());
    assert!(parse_serve_args(&strings(&["--help"]), None, None)
        .unwrap()
        .is_none());
}
