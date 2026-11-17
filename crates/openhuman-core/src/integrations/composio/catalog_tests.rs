use super::*;

fn contract(slug: &str, toolkit: &str, required: &[&str], fields: &[&str]) -> ToolContract {
    ToolContract {
        slug: slug.into(),
        toolkit: toolkit.into(),
        description: None,
        required_args: required.iter().map(|s| s.to_string()).collect(),
        input_schema: None,
        output_fields: fields.iter().map(|s| s.to_string()).collect(),
        output_schema: if fields.is_empty() {
            None
        } else {
            Some(serde_json::json!({"type": "object"}))
        },
        primary_array_path: None,
        is_curated: false,
    }
}

#[tokio::test]
async fn a_seeded_catalog_answers_without_calling_the_module() {
    seed_live_catalog_cache(
        "hostseam1",
        vec![contract("HOSTSEAM1_A", "hostseam1", &["x"], &[])],
    );
    let got = fetch_live_toolkit_catalog(&Config::default(), "HOSTSEAM1")
        .await
        .expect("seeded");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].slug, "HOSTSEAM1_A");
}

#[tokio::test]
async fn required_args_and_response_fields_project_the_catalog() {
    seed_live_catalog_cache(
        "hostseam2",
        vec![
            contract("HOSTSEAM2_LIST", "hostseam2", &["q"], &["items"]),
            contract("HOSTSEAM2_BARE", "hostseam2", &[], &[]),
        ],
    );
    let cfg = Config::default();
    assert_eq!(
        composio_required_args(&cfg, "hostseam2_list").await,
        Some(vec!["q".to_string()])
    );
    assert_eq!(
        composio_response_fields(&cfg, "HOSTSEAM2_LIST").await,
        Some(vec!["items".to_string()])
    );
    // Real action with no published output schema: unknown, not empty.
    assert_eq!(composio_response_fields(&cfg, "HOSTSEAM2_BARE").await, None);
    assert_eq!(
        composio_required_args(&cfg, "HOSTSEAM2_MISSING").await,
        None
    );
}

#[tokio::test]
async fn a_seeded_probe_is_overlaid_onto_the_contract() {
    seed_live_catalog_cache(
        "hostseam3",
        vec![contract("HOSTSEAM3_X", "hostseam3", &[], &["a"])],
    );
    seed_probe_cache(
        "HOSTSEAM3_X",
        ProbedOutputSample {
            primary_array_path: Some("data.rows".into()),
            output_fields: vec!["rows".into()],
            sample: serde_json::Value::Null,
        },
    );
    let got = fetch_live_toolkit_catalog(&Config::default(), "hostseam3")
        .await
        .unwrap();
    assert_eq!(got[0].primary_array_path.as_deref(), Some("data.rows"));
    assert_eq!(got[0].output_fields, vec!["rows".to_string()]);
}
