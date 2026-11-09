use serde_json::json;

use super::render::render_table;

#[test]
fn table_groups_by_check_category_and_renders_fix_hints() {
    let report = json!({
        "findings": [
            {
                "check_id": "config/provenance",
                "severity": "warn",
                "message": "The setting came from an implicit source.",
                "fix_hint": "Set the intended source explicitly.",
                "duration_ms": 1,
                "affects": []
            },
            {
                "check_id": "workspace/layout",
                "severity": "pass",
                "message": "The workspace layout is valid.",
                "fix_hint": "No action required.",
                "duration_ms": 1,
                "affects": []
            },
            {
                "check_id": "config/file-integrity",
                "severity": "error",
                "message": "The config file is invalid.",
                "fix_hint": "Repair the config file.",
                "duration_ms": 1,
                "affects": []
            }
        ],
        "summary": { "pass": 1, "info": 0, "warn": 1, "error": 1, "skipped": 0 },
        "ok": false
    });

    let rendered = render_table(&report);

    assert!(rendered.contains("config"));
    assert!(rendered.contains("workspace"));
    assert!(rendered.contains("[warn] config/provenance The setting came from an implicit source."));
    assert!(rendered.contains("[error] config/file-integrity The config file is invalid."));
    assert!(rendered.contains("  fix: Set the intended source explicitly."));
    assert!(rendered.contains("  fix: Repair the config file."));
    assert!(rendered.contains("Summary:"));
    let first_config = rendered.find("config/provenance").unwrap();
    let second_config = rendered.find("config/file-integrity").unwrap();
    let workspace = rendered.find("workspace/layout").unwrap();
    assert!(first_config < second_config && second_config < workspace);
}

#[test]
fn table_header_renders_report_profile_provenance_without_inference() {
    let report = json!({
        "profile": "desktop-default",
        "profile_layout_version": 2,
        "config_resolution_source": "active_user.toml",
        "findings": [],
        "summary": { "pass": 0, "info": 0, "warn": 0, "error": 0, "skipped": 0 },
        "ok": true
    });

    let rendered = render_table(&report);

    assert!(rendered
        .starts_with("Profile: desktop-default | layout v2 | config source: active_user.toml\n"));
}
