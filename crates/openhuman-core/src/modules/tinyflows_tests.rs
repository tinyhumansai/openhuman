use super::*;

#[test]
fn the_disabled_message_names_the_component_and_the_switch() {
    let message = disabled_message();
    assert!(
        message.starts_with("COMPONENT_DISABLED:tinyflows"),
        "{message}"
    );
    assert!(message.contains("[components] tinyflows = true"));
}

#[test]
fn the_module_is_available_only_while_its_component_is_on() {
    let mut config = Config::default();
    assert_eq!(available(&config), cfg!(feature = "modules"));
    config.components.tinyflows = false;
    assert!(!available(&config));
}

#[tokio::test]
async fn a_switched_off_component_never_loads_the_module() {
    let mut config = Config::default();
    config.components.tinyflows = false;
    let error = proxy(&config).await.unwrap_err();
    assert!(error.starts_with("COMPONENT_DISABLED:tinyflows"), "{error}");
    let error = run_proxy(&config).await.unwrap_err();
    assert!(error.starts_with("COMPONENT_DISABLED:tinyflows"), "{error}");
}

#[tokio::test]
async fn the_schedule_object_answers_on_the_contracts_names() {
    let config = Config::default();
    let proxy = proxy(&config).await.expect("module attaches");
    let millis: i64 = proxy
        .call(
            tinyflows_bus::schedule::members::PARSE_HUMAN_DELAY,
            ("30m",),
        )
        .await
        .unwrap();
    assert_eq!(millis, 30 * 60 * 1000);
}
