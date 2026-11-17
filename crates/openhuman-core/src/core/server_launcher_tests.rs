use super::*;

fn ok_launcher(_: ServeRequest) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
    Box::pin(async { Ok(()) })
}

fn other_launcher(_: ServeRequest) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
    Box::pin(async { anyhow::bail!("second launcher must not replace the first") })
}

#[tokio::test]
async fn first_installed_launcher_wins() {
    install_server_launcher(ok_launcher);
    install_server_launcher(other_launcher);
    let launcher = installed_server_launcher().expect("a launcher is installed");
    let request = ServeRequest {
        host: None,
        port: None,
        socketio_enabled: true,
        headless_api: false,
        mode: crate::core::runtime::Mode::SingleUser,
        saas_config: None,
        host_boot: None,
    };
    launcher(request).await.expect("the first launcher runs");
}

#[test]
fn the_default_is_single_user() {
    let (mode, config) = resolve_mode(None, None, None).unwrap();
    assert_eq!(mode, crate::core::runtime::Mode::SingleUser);
    assert!(config.is_none());
}

#[test]
fn saas_needs_an_operator_config() {
    assert!(resolve_mode(Some("saas"), None, None).is_err());
    let (mode, config) = resolve_mode(Some("saas"), None, Some("/srv/oh.toml".into())).unwrap();
    assert_eq!(mode, crate::core::runtime::Mode::Saas);
    assert_eq!(config, Some(std::path::PathBuf::from("/srv/oh.toml")));
}

#[test]
fn the_environment_can_raise_the_mode_but_not_lower_it() {
    use crate::core::runtime::Mode;
    let config = Some(std::path::PathBuf::from("/srv/oh.toml"));
    let (mode, _) = resolve_mode(Some("single-user"), Some("saas"), config.clone()).unwrap();
    assert_eq!(mode, Mode::Saas);
    let (mode, _) = resolve_mode(Some("saas"), Some("single-user"), config).unwrap();
    assert_eq!(mode, Mode::Saas);
}

#[test]
fn a_saas_config_without_saas_is_refused() {
    let err = resolve_mode(None, None, Some("/srv/oh.toml".into())).unwrap_err();
    assert!(err.to_string().contains("--mode saas"), "{err}");
}

#[test]
fn an_unknown_mode_is_refused_with_its_source() {
    let err = resolve_mode(Some("multi"), None, None).unwrap_err();
    assert!(err.to_string().contains("--mode"), "{err}");
    let err = resolve_mode(None, Some("multi"), None).unwrap_err();
    assert!(err.to_string().contains("OPENHUMAN_MODE"), "{err}");
}

#[test]
fn host_boot_hands_a_value_over_once() {
    let boot = HostBoot::new(String::from("builder"));
    let copy = boot.clone();
    assert_eq!(boot, copy, "clones share one slot");
    assert_eq!(boot.take::<String>().as_deref(), Some("builder"));
    assert!(copy.take::<String>().is_none(), "taken once");
}

#[test]
fn host_boot_keeps_the_value_when_the_type_does_not_match() {
    let boot = HostBoot::new(7u32);
    assert!(boot.take::<String>().is_none());
    assert_eq!(
        boot.take::<u32>(),
        Some(7),
        "a wrong guess does not consume it"
    );
}
