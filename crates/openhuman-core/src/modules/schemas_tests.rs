use super::{
    all_controller_schemas, all_registered_controllers, controller_value, schemas, string_param,
};
use crate::core::runtime::context::CoreContext;
use crate::core::runtime::DispatchCtx;
use crate::core::runtime::DomainSet;
use serde_json::{Map, Value};

#[test]
fn every_schema_is_in_the_modules_namespace() {
    for schema in all_controller_schemas() {
        assert_eq!(schema.namespace, "modules");
        assert_ne!(
            schema.function, "unknown",
            "an advertised function fell through to the unknown arm"
        );
    }
}

#[test]
fn registered_controllers_match_the_advertised_schemas() {
    // Two lists that must agree: one drives `/schema`, the other dispatch.
    let advertised: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.function)
        .collect();
    let registered: Vec<&str> = all_registered_controllers()
        .iter()
        .map(|c| c.schema.function)
        .collect();
    assert_eq!(advertised, registered);
}

#[test]
fn an_unknown_function_falls_through_to_the_unknown_arm() {
    assert_eq!(schemas("nope").function, "unknown");
}

#[test]
fn a_blank_or_missing_id_is_not_a_parameter() {
    let mut params = Map::new();
    assert_eq!(string_param(&params, "id"), None);
    params.insert("id".to_string(), Value::String("   ".to_string()));
    assert_eq!(string_param(&params, "id"), None);
    params.insert("id".to_string(), Value::String(" tinydocs ".to_string()));
    assert_eq!(string_param(&params, "id"), Some("tinydocs".to_string()));
}

#[test]
fn controller_value_reports_serialization_errors() {
    let invalid_object_key = std::collections::HashMap::from([(vec![1, 2], true)]);

    let error = controller_value(invalid_object_key).unwrap_err();

    assert!(error.contains("key must be a string"));
}

#[tokio::test]
async fn computer_status_handler_returns_serialized_status() {
    let ctx =
        CoreContext::for_test_with_config(DomainSet::full(), crate::config::Config::default());

    let status = CoreContext::scope(ctx.clone(), async {
        super::handle_computer_status(DispatchCtx::new(ctx.clone()), Map::new()).await
    })
    .await
    .expect("computer status should serialize");

    assert!(status.get("decision_model").is_some());
}

#[tokio::test]
async fn forgetting_sites_reports_how_many_went() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = crate::config::Config::default();
    config.workspace_dir = dir.path().join("workspace");
    let sites = crate::modules::computer_config::trace_dir(&config).join("sites");
    std::fs::create_dir_all(&sites).unwrap();
    for site in ["shop.test", "books.test"] {
        std::fs::write(sites.join(format!("{site}.json")), b"{}").unwrap();
    }
    let ctx = CoreContext::for_test_with_config(DomainSet::full(), config);

    let (one, every) = CoreContext::scope(ctx.clone(), async {
        let mut one = Map::new();
        one.insert(
            "site".to_string(),
            Value::String("https://www.shop.test/cart".to_string()),
        );
        let one = super::handle_browser_forget_sites(DispatchCtx::new(ctx.clone()), one).await;
        (
            one,
            super::handle_browser_forget_sites(DispatchCtx::new(ctx.clone()), Map::new()).await,
        )
    })
    .await;

    assert_eq!(one.unwrap()["forgotten"], 1);
    assert_eq!(every.unwrap()["forgotten"], 1);
    assert!(!sites.join("books.test.json").exists());
}

#[test]
fn the_chrome_check_says_where_to_fix_a_missing_chrome() {
    let missing = super::readiness_error(
        "BrowserUnavailable: browser unavailable: Chrome not found. Checked: …",
    );
    assert!(missing.starts_with("Chrome was not found."), "{missing}");
    assert!(missing.contains("Chrome path"), "{missing}");
    assert_eq!(
        super::readiness_error("Bus: launch timed out"),
        "Chrome could not start: Bus: launch timed out"
    );
    // TinyComputer 0.11's own words for each.
    let missing = super::readiness_error(
        "BrowserUnavailable: browser unavailable: no Chrome or Chromium was found on this machine; give the path of the browser to use",
    );
    assert!(missing.starts_with("Chrome was not found."), "{missing}");
    let unusable = super::readiness_error(
        "BrowserUnavailable: browser unavailable: the browser binary given could not be started (Chrome exited before providing DevTools URL); check that its path names Chrome or Chromium",
    );
    assert!(
        unusable.starts_with("Chrome could not be started from Chrome path."),
        "{unusable}"
    );
}

#[tokio::test]
async fn a_blank_or_unreadable_site_forgets_nothing() {
    for site in [Value::String("  ".to_string()), Value::from(7)] {
        let mut params = Map::new();
        params.insert("site".to_string(), site);
        let error = super::handle_browser_forget_sites(DispatchCtx::process_default(), params)
            .await
            .unwrap_err();
        assert!(error.contains("must name a site"), "{error}");
    }
}
