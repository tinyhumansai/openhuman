use super::*;

fn sample_snapshot() -> ConfigSnapshotFields {
    ConfigSnapshotFields {
        config: json!({
            "api_url": "https://api.example.com",
            "default_model": "gpt-4",
            "default_temperature": 0.7,
            "memory": {"enabled": true, "limit": 1000},
            "runtime": {"debug": false, "workers": 4},
            "browser": {"allow_all": false},
        }),
        workspace_dir: "/tmp/ws".into(),
        config_path: "/tmp/config.toml".into(),
    }
}

#[test]
fn model_section_projects_model_fields() {
    let snap = sample_snapshot();
    let v = settings_section_json("model", &snap, vec!["a".into()]);
    assert_eq!(v["result"]["section"], "model");
    assert_eq!(v["result"]["settings"]["default_model"], "gpt-4");
    assert_eq!(v["result"]["workspace_dir"], "/tmp/ws");
    assert_eq!(v["result"]["config_path"], "/tmp/config.toml");
    assert_eq!(v["logs"], json!(["a"]));
}

#[test]
fn memory_section_returns_memory_object() {
    let snap = sample_snapshot();
    let v = settings_section_json("memory", &snap, vec![]);
    assert_eq!(v["result"]["settings"]["enabled"], true);
    assert_eq!(v["result"]["settings"]["limit"], 1000);
}

#[test]
fn runtime_section_returns_runtime_object() {
    let snap = sample_snapshot();
    let v = settings_section_json("runtime", &snap, vec![]);
    assert_eq!(v["result"]["settings"]["debug"], false);
    assert_eq!(v["result"]["settings"]["workers"], 4);
}

#[test]
fn browser_section_returns_browser_object() {
    let snap = sample_snapshot();
    let v = settings_section_json("browser", &snap, vec![]);
    assert_eq!(v["result"]["settings"]["allow_all"], false);
}

#[test]
fn unknown_section_returns_null_settings() {
    let snap = sample_snapshot();
    let v = settings_section_json("no_such", &snap, vec![]);
    assert!(v["result"]["settings"].is_null());
    assert_eq!(v["result"]["section"], "no_such");
}

#[test]
fn logs_are_always_passed_through() {
    let snap = sample_snapshot();
    let logs = vec!["one".to_string(), "two".to_string()];
    let v = settings_section_json("model", &snap, logs.clone());
    assert_eq!(v["logs"], json!(logs));
}

#[test]
fn missing_section_fields_become_null() {
    let snap = ConfigSnapshotFields {
        config: json!({}),
        workspace_dir: "/tmp/ws".into(),
        config_path: "/tmp/cfg.toml".into(),
    };
    let v = settings_section_json("memory", &snap, vec![]);
    assert!(v["result"]["settings"].is_null());
}

#[test]
fn model_section_missing_fields_yields_null_entries() {
    let snap = ConfigSnapshotFields {
        config: json!({ "default_model": "gpt-4" }),
        workspace_dir: "/tmp/ws".into(),
        config_path: "/tmp/cfg.toml".into(),
    };
    let v = settings_section_json("model", &snap, vec![]);
    // `default_model` present; the others (api_url/default_temperature) null.
    assert_eq!(v["result"]["settings"]["default_model"], "gpt-4");
    assert!(v["result"]["settings"]["api_url"].is_null());
}

#[test]
fn section_is_echoed_back_verbatim() {
    let snap = sample_snapshot();
    let sections = ["model", "memory", "runtime", "browser", "whatever"];
    for s in sections {
        let v = settings_section_json(s, &snap, vec![]);
        assert_eq!(v["result"]["section"], s);
    }
}
