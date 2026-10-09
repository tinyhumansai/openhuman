use super::*;
use serde_json::json;

#[test]
fn meta_keys_include_ui_members() {
    let keys = meta_keys(&json!({"ui": {"resourceUri": "ui://a", "visibility": ["app"]}, "x": 1}));
    assert!(keys.contains(&"ui".to_string()));
    assert!(keys.contains(&"ui.resourceUri".to_string()));
    assert!(keys.contains(&"x".to_string()));
}
