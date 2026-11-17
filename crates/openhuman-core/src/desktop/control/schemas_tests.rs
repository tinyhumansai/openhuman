use super::*;
use crate::core::runtime::context::CoreContext;
use crate::core::runtime::DispatchCtx;
use crate::core::runtime::DomainSet;

fn controller(name: &str) -> RegisteredController {
    all_registered_controllers()
        .into_iter()
        .find(|item| item.schema.function == name)
        .unwrap()
}

#[tokio::test]
async fn desktop_controllers_expose_expected_rpc_contracts_and_dispatch_pending() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 5);
    for item in &controllers {
        assert_eq!(item.schema.namespace, "desktop");
        assert!(item.rpc_method_name().starts_with("openhuman.desktop_"));
    }
    assert_eq!(controller("set_enabled").schema.inputs.len(), 1);
    assert_eq!(controller("confirm").schema.inputs.len(), 2);
}

fn scoped_context(computer: bool) -> (tempfile::TempDir, std::sync::Arc<CoreContext>) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.components.computer = computer;
    (
        dir,
        CoreContext::for_test_with_config(DomainSet::full(), config),
    )
}

#[tokio::test]
async fn pending_lists_actions_while_the_computer_component_is_on() {
    let (_dir, context) = scoped_context(true);
    CoreContext::scope(context.clone(), async {
        let value = (controller("pending").handler)(DispatchCtx::new(context.clone()), Map::new())
            .await
            .unwrap();
        assert!(value.is_array());
    })
    .await;
}

#[tokio::test]
async fn every_desktop_controller_answers_component_disabled_when_computer_is_off() {
    let (_dir, context) = scoped_context(false);
    CoreContext::scope(context.clone(), async {
        for function in ["status", "probe", "pending"] {
            let error =
                (controller(function).handler)(DispatchCtx::new(context.clone()), Map::new())
                    .await
                    .unwrap_err();
            assert!(
                error.contains("COMPONENT_DISABLED:computer"),
                "{function}: {error}"
            );
        }
        let mut params = Map::new();
        params.insert("enabled".to_owned(), Value::Bool(true));
        let error = (controller("set_enabled").handler)(DispatchCtx::new(context.clone()), params)
            .await
            .unwrap_err();
        assert!(error.contains("COMPONENT_DISABLED:computer"), "{error}");
        let mut params = Map::new();
        params.insert("confirmation_id".to_owned(), Value::String("id".to_owned()));
        params.insert("approve".to_owned(), Value::Bool(true));
        let error = (controller("confirm").handler)(DispatchCtx::new(context.clone()), params)
            .await
            .unwrap_err();
        assert!(error.contains("COMPONENT_DISABLED:computer"), "{error}");
    })
    .await;
}

#[tokio::test]
async fn malformed_controller_inputs_fail_before_loading_config_or_acting() {
    for invalid in [
        Value::Null,
        Value::String("true".to_owned()),
        Value::Number(1.into()),
    ] {
        let mut params = Map::new();
        params.insert("enabled".to_owned(), invalid);
        assert_eq!(
            (controller("set_enabled").handler)(DispatchCtx::process_default(), params)
                .await
                .unwrap_err(),
            "enabled must be a boolean"
        );
    }
    let mut params = Map::new();
    params.insert("confirmation_id".to_owned(), Value::String(String::new()));
    params.insert("approve".to_owned(), Value::Bool(true));
    assert_eq!(
        (controller("confirm").handler)(DispatchCtx::process_default(), params)
            .await
            .unwrap_err(),
        "confirmation_id is required"
    );
    let mut params = Map::new();
    params.insert("confirmation_id".to_owned(), Value::String("id".to_owned()));
    params.insert("approve".to_owned(), Value::String("yes".to_owned()));
    assert_eq!(
        (controller("confirm").handler)(DispatchCtx::process_default(), params)
            .await
            .unwrap_err(),
        "approve must be a boolean"
    );
}

#[tokio::test]
async fn controllers_use_scoped_config_for_disabled_desktop_and_reject_unknown_confirmation() {
    let _loopback = super::super::ops::test_loopback_guard();
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    let context = CoreContext::for_test_with_config(DomainSet::full(), config);

    CoreContext::scope(context.clone(), async {
        let status = (controller("status").handler)(DispatchCtx::new(context.clone()), Map::new())
            .await
            .unwrap();
        assert_eq!(status["enabled"], false);

        let probe = (controller("probe").handler)(DispatchCtx::new(context.clone()), Map::new())
            .await
            .unwrap();
        assert_eq!(probe["ok"], false);
        assert!(probe["reason"].as_str().unwrap().contains("disabled"));

        let mut params = Map::new();
        params.insert("enabled".to_owned(), Value::Bool(false));
        let disabled =
            (controller("set_enabled").handler)(DispatchCtx::new(context.clone()), params)
                .await
                .unwrap();
        assert_eq!(disabled["enabled"], false);

        let mut params = Map::new();
        params.insert(
            "confirmation_id".to_owned(),
            Value::String("missing".to_owned()),
        );
        params.insert("approve".to_owned(), Value::Bool(true));
        let error = (controller("confirm").handler)(DispatchCtx::new(context.clone()), params)
            .await
            .unwrap_err();
        assert!(error.contains("missing or expired"));
    })
    .await;
}
