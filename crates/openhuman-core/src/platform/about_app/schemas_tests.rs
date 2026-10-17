use super::*;

#[test]
fn schema_names_are_stable() {
    let list = about_app_schemas("about_app_list");
    assert_eq!(list.namespace, "about_app");
    assert_eq!(list.function, "list");

    let lookup = about_app_schemas("about_app_lookup");
    assert_eq!(lookup.namespace, "about_app");
    assert_eq!(lookup.function, "lookup");

    let search = about_app_schemas("about_app_search");
    assert_eq!(search.namespace, "about_app");
    assert_eq!(search.function, "search");
}

#[test]
fn controller_lists_match_lengths() {
    assert_eq!(
        all_about_app_controller_schemas().len(),
        all_about_app_registered_controllers().len()
    );
}

#[tokio::test]
async fn rendered_catalog_responses_omit_retired_promises_and_keep_real_capabilities() {
    use crate::core::runtime::{CoreContext, DomainSet};
    let ctx = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None));
    let retired = [
        "workflows.connect_google",
        "workflows.connect_notion",
        "workflows.connect_crypto_exchange",
        "settings.delete_all_data",
    ];
    for category in [None, Some("workflows"), Some("settings")] {
        let mut params = Map::new();
        if let Some(category) = category {
            params.insert("category".into(), Value::String(category.into()));
        }
        let response = handle_about_app_list(ctx.clone(), params).await.unwrap();
        let rendered = response.to_string();
        for id in retired {
            assert!(!rendered.contains(id), "rendered list must omit {id}");
        }
        let rows = crate::core::unwrap_rpc(&response)
            .as_array()
            .expect("capability list");
        if category != Some("workflows") {
            assert!(rows
                .iter()
                .any(|row| row["id"] == "settings.clear_app_data"));
        }
        if category != Some("settings") {
            assert!(rows.iter().any(|row| row["id"] == "composio.direct_mode"));
        }
    }
    for id in retired {
        let params = serde_json::json!({"id":id}).as_object().unwrap().clone();
        assert!(handle_about_app_lookup(ctx.clone(), params)
            .await
            .unwrap_err()
            .contains("unknown capability id"));
        let params = serde_json::json!({"query":id}).as_object().unwrap().clone();
        let response = handle_about_app_search(ctx.clone(), params).await.unwrap();
        assert!(crate::core::unwrap_rpc(&response)
            .as_array()
            .unwrap()
            .is_empty());
    }
    for (query, id) in [
        ("Composio", "composio.direct_mode"),
        ("clear app data", "settings.clear_app_data"),
    ] {
        let params = serde_json::json!({"query":query})
            .as_object()
            .unwrap()
            .clone();
        let response = handle_about_app_search(ctx.clone(), params).await.unwrap();
        assert!(crate::core::unwrap_rpc(&response)
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == id));
    }
}
