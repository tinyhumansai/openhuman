use super::*;

#[test]
fn for_connected_returns_none_when_no_toolkits() {
    assert!(SkillDelegationTool::for_connected(vec![]).is_none());
}

#[test]
fn for_connected_uses_canonical_tool_name() {
    let tool = SkillDelegationTool::for_connected(vec![(
        "gmail".to_string(),
        "Email access.".to_string(),
    )])
    .unwrap();
    assert_eq!(tool.name(), INTEGRATIONS_DELEGATE_TOOL_NAME);
    assert_eq!(tool.name(), "delegate_to_integrations_agent");
}

#[test]
fn description_enumerates_connected_toolkits() {
    let tool = SkillDelegationTool::for_connected(vec![
        ("gmail".to_string(), "Email access.".to_string()),
        ("notion".to_string(), "Pages and databases.".to_string()),
    ])
    .unwrap();
    let desc = tool.description();
    assert!(desc.contains("gmail"));
    assert!(desc.contains("notion"));
    assert!(desc.contains("Email access."));
    assert!(desc.contains("Pages and databases."));
}

#[test]
fn parameters_schema_enforces_toolkit_enum_against_connected_slugs() {
    let tool = SkillDelegationTool::for_connected(vec![
        ("gmail".to_string(), "Email.".to_string()),
        ("notion".to_string(), "Docs.".to_string()),
    ])
    .unwrap();
    let schema = tool.parameters_schema();
    let enum_vals = schema["properties"]["toolkit"]["enum"]
        .as_array()
        .expect("toolkit enum is an array");
    let collected: Vec<&str> = enum_vals.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(collected, vec!["gmail", "notion"]);

    let required = schema["required"].as_array().expect("required is an array");
    let required: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(required.contains(&"toolkit"));
    assert!(required.contains(&"prompt"));
}

#[tokio::test]
async fn execute_rejects_missing_toolkit_argument() {
    let tool =
        SkillDelegationTool::for_connected(vec![("gmail".to_string(), "Email.".to_string())])
            .unwrap();
    let result = tool.execute(json!({"prompt": "x"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("toolkit"));
}

#[tokio::test]
async fn execute_rejects_unknown_toolkit_with_allowed_list() {
    // Force the live re-check to return "Unavailable" so the test never
    // reads host config or reaches the Composio backend — the reject must
    // come purely from the in-memory snapshot (gmail/notion, no slack).
    set_live_fetch_override(None);
    let tool = SkillDelegationTool::for_connected(vec![
        ("gmail".to_string(), "Email.".to_string()),
        ("notion".to_string(), "Docs.".to_string()),
    ])
    .unwrap();
    let result = tool
        .execute(json!({"toolkit": "slack", "prompt": "hi"}))
        .await
        .unwrap();
    clear_live_fetch_override();
    assert!(result.is_error);
    let body = result.output();
    assert!(body.contains("slack"));
    assert!(body.contains("gmail"));
    assert!(body.contains("notion"));
}

#[tokio::test]
async fn execute_rejects_empty_prompt() {
    let tool =
        SkillDelegationTool::for_connected(vec![("gmail".to_string(), "Email.".to_string())])
            .unwrap();
    let result = tool
        .execute(json!({"toolkit": "gmail", "prompt": "   "}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("prompt"));
}

#[tokio::test]
async fn execute_normalises_toolkit_input_before_matching() {
    // Mixed-case + odd-character user input must collapse onto the
    // canonical slug before the connectedness check fires.
    // Pin the live re-check to the same snapshot so the test is hermetic
    // (no host config / backend read): `gmail` stays unknown, while the
    // normalised `google_calendar` is accepted.
    set_live_fetch_override(Some(vec!["google_calendar".to_string()]));
    let tool = SkillDelegationTool::for_connected(vec![(
        "google_calendar".to_string(),
        "Calendar.".to_string(),
    )])
    .unwrap();
    // "GMail" sanitises to `gmail` — NOT in the connected set, so it
    // must be rejected with the unknown-toolkit message that
    // enumerates the allowed slugs.
    let bad = tool
        .execute(json!({"toolkit": "GMail", "prompt": "x"}))
        .await
        .unwrap();
    assert!(bad.is_error);
    let bad_body = bad.output();
    assert!(
        bad_body.contains("not connected"),
        "expected unknown-toolkit error path, got: {bad_body}"
    );
    assert!(bad_body.contains("google_calendar"));

    // "Google-Calendar" sanitises to `google_calendar`, which IS in
    // the connected set, so the toolkit gate must let it through.
    // Dispatch will then fail because no agent registry is wired up
    // in this unit-test process — but the error must NOT be the
    // unknown-toolkit branch, because that branch was supposed to
    // be bypassed by the slug normalisation.
    let ok = tool
        .execute(json!({"toolkit": "Google-Calendar", "prompt": "do thing"}))
        .await;
    match ok {
        Ok(result) => {
            let body = result.output();
            assert!(
                !body.contains("not connected"),
                "normalised slug should pass the toolkit gate, got: {body}"
            );
        }
        Err(err) => {
            let msg = err.to_string();
            assert!(
                !msg.contains("not connected"),
                "normalised slug should pass the toolkit gate, got: {msg}"
            );
        }
    }
    clear_live_fetch_override();
}

#[test]
fn resolve_connected_toolkits_prefers_live_recheck_for_unknown_slug() {
    let snapshot = vec![("gmail".to_string(), "Email".to_string())];

    let (known_snapshot, allowed_snapshot) = resolve_connected_toolkits(&snapshot, "gmail", None);
    assert!(known_snapshot);
    assert_eq!(allowed_snapshot, vec!["gmail".to_string()]);

    let live = vec!["gmail".to_string(), "notion".to_string()];
    let (known_live, allowed_live) =
        resolve_connected_toolkits(&snapshot, "notion", Some(live.as_slice()));
    assert!(known_live);
    assert_eq!(allowed_live, live);

    let live_no_match = vec!["gmail".to_string(), "notion".to_string()];
    let (known_none, allowed_none) =
        resolve_connected_toolkits(&snapshot, "slack", Some(live_no_match.as_slice()));
    assert!(!known_none);
    assert_eq!(allowed_none, vec!["gmail".to_string()]);
}
