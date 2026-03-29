//! Settings “section” views for the core CLI (slice full config JSON by area).

use serde_json::json;

/// Fields matching the config snapshot payload shape used by RPC/CLI.
#[derive(Debug, Clone)]
pub struct ConfigSnapshotFields {
    pub config: serde_json::Value,
    pub workspace_dir: String,
    pub config_path: String,
}

/// Build `{ section, settings, workspace_dir, config_path }` plus caller-supplied logs.
pub fn settings_section_json(
    section: &str,
    snap: &ConfigSnapshotFields,
    logs: Vec<String>,
) -> serde_json::Value {
    let cfg = &snap.config;
    let settings = match section {
        "model" => json!({
            "api_key": cfg.get("api_key"),
            "api_url": cfg.get("api_url"),
            "default_provider": cfg.get("default_provider"),
            "default_model": cfg.get("default_model"),
            "default_temperature": cfg.get("default_temperature"),
        }),
        "memory" => cfg
            .get("memory")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "tunnel" => cfg
            .get("tunnel")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "runtime" => cfg
            .get("runtime")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "browser" => cfg
            .get("browser")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        _ => serde_json::Value::Null,
    };

    json!({
        "result": {
            "section": section,
            "settings": settings,
            "workspace_dir": snap.workspace_dir,
            "config_path": snap.config_path,
        },
        "logs": logs
    })
}
