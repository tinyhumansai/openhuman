use super::*;

#[test]
fn no_inputs_returns_header_only() {
    let out = render_workflow_toml("my-skill", "Does the thing.", &[]);
    assert!(out.contains("id = \"my-skill\""));
    assert!(out.contains("when_to_use = \"Does the thing.\""));
    assert!(!out.contains("[[inputs]]"));
}

#[test]
fn one_input_with_all_fields_roundtrips() {
    let inputs = vec![WorkflowCreateInputDef {
        name: "repo".into(),
        description: Some("owner/name".into()),
        required: true,
        type_: Some("string".into()),
    }];
    let out = render_workflow_toml("my-skill", "Does the thing.", &inputs);
    // Parse it back through the actual TOML parser to prove the
    // output is well-formed — the registry uses `toml::from_str` so
    // any round-trip failure here would surface at skill discovery.
    let parsed: toml::Value = toml::from_str(&out).expect("emitted skill.toml must parse");
    let inputs_arr = parsed["inputs"].as_array().expect("[[inputs]] is an array");
    assert_eq!(inputs_arr.len(), 1);
    let entry = &inputs_arr[0];
    assert_eq!(entry["name"].as_str(), Some("repo"));
    assert_eq!(entry["description"].as_str(), Some("owner/name"));
    assert_eq!(entry["required"].as_bool(), Some(true));
    assert_eq!(entry["type"].as_str(), Some("string"));
}

#[test]
fn optional_fields_omitted_when_empty() {
    let inputs = vec![WorkflowCreateInputDef {
        name: "n".into(),
        description: None,
        required: false,
        type_: None,
    }];
    let out = render_workflow_toml("my-skill", "x", &inputs);
    let parsed: toml::Value = toml::from_str(&out).expect("parse");
    let entry = &parsed["inputs"].as_array().unwrap()[0];
    assert_eq!(entry["name"].as_str(), Some("n"));
    assert_eq!(entry["required"].as_bool(), Some(false));
    assert!(entry.get("description").is_none());
    assert!(entry.get("type").is_none());
}

#[test]
fn escapes_dangerous_chars_in_strings() {
    let inputs = vec![WorkflowCreateInputDef {
        name: "n".into(),
        description: Some("has \"quotes\" and \\ backslash\nand newline".into()),
        required: true,
        type_: None,
    }];
    let out = render_workflow_toml("my-skill", "x", &inputs);
    // Must still parse cleanly — the escape logic is what we're
    // exercising here; the round-trip assertion below is the contract.
    let parsed: toml::Value = toml::from_str(&out).expect("escaped strings must parse");
    let entry = &parsed["inputs"].as_array().unwrap()[0];
    assert_eq!(
        entry["description"].as_str(),
        Some("has \"quotes\" and \\ backslash\nand newline")
    );
}

/// The trigger half of mid-session refresh: creating a workflow must
/// publish `DomainEvent::WorkflowsChanged` so live sessions re-scan. This
/// guards the `publish_global` emission line (the `refresh_workflows` test
/// writes to disk directly and bypasses create/install, so without this a
/// dropped emission would stay green while silently killing the feature).
#[tokio::test]
async fn create_workflow_inner_emits_workflows_changed() {
    use crate::core::events::DomainEvent;
    use tinybus::TryRecvError;

    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS
        .get()
        .expect("event bus should be initialized")
        .receiver();

    let home = tempfile::TempDir::new().expect("temp home");
    let ws = tempfile::TempDir::new().expect("temp workspace");
    let params = CreateWorkflowParams {
        name: "zz-emit-test".into(),
        description: "emit test skill".into(),
        scope: WorkflowScope::User,
        ..Default::default()
    };
    create_workflow_inner(Some(home.path()), ws.path(), params)
        .expect("create_workflow_inner should succeed");

    let mut saw = false;
    loop {
        match rx.try_recv() {
            // The event bus is a process-wide singleton, so other tests
            // running in parallel publish their own WorkflowsChanged events
            // (e.g. "install"/"uninstall" from ops_install). Match only our
            // own "create" reason and skip the rest rather than asserting on
            // whichever event happens to arrive first.
            Ok(DomainEvent::WorkflowsChanged { reason }) if reason == "create" => {
                saw = true;
                break;
            }
            Ok(_) => continue,
            Err(TryRecvError::Lagged(_)) => continue,
            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
        }
    }
    assert!(
        saw,
        "create_workflow_inner must publish DomainEvent::WorkflowsChanged"
    );
}
