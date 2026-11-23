use super::*;
use serde_json::json;

/// Every action that can change a page or act for the signed-in user.
const REFUSED: &[&str] = &[
    "click",
    "double_click",
    "fill",
    "type",
    "press",
    "select",
    "check",
    "task",
    "task_continue",
    "confirm_pending",
    "wait_download",
];

/// Reading, looking and leaving.
const ALLOWED: &[&str] = &[
    "open",
    "snapshot",
    "read_page",
    "get_text",
    "get_title",
    "get_url",
    "wait",
    "hover",
    "scroll",
    "is_visible",
    "list_downloads",
    "task_cancel",
    "close",
];

#[test]
fn every_acting_action_is_refused_by_name() {
    for action in REFUSED {
        let refusal = view_only_refusal(action, &json!({"action": action}))
            .unwrap_or_else(|| panic!("{action} must be refused"));
        assert!(
            refusal.contains(&format!("`{action}`")),
            "{action}: {refusal}"
        );
        assert!(refusal.contains("view-only"), "{action}: {refusal}");
    }
}

#[test]
fn every_reading_action_passes() {
    for action in ALLOWED {
        assert_eq!(
            view_only_refusal(action, &json!({"action": action})),
            None,
            "{action}"
        );
    }
}

#[test]
fn find_is_refused_only_when_it_clicks_or_fills() {
    for follow in ["click", "fill", " Click "] {
        let args = json!({"action": "find", "by": "text", "value": "Go", "find_action": follow});
        let refusal = view_only_refusal("find", &args)
            .unwrap_or_else(|| panic!("find with {follow:?} must be refused"));
        assert!(refusal.contains("`find`"), "{refusal}");
        assert!(refusal.contains("view-only"), "{refusal}");
    }
    for follow in [json!("text"), json!("hover"), json!(null)] {
        let args = json!({"action": "find", "by": "text", "value": "Go", "find_action": follow});
        assert_eq!(view_only_refusal("find", &args), None, "{follow}");
    }
    assert_eq!(
        view_only_refusal("find", &json!({"action": "find", "by": "text", "value": "Go"})),
        None
    );
}

#[test]
fn an_unknown_action_is_left_to_normal_handling() {
    for action in ["drag", "", "evaluate"] {
        assert_eq!(view_only_refusal(action, &json!({"action": action})), None);
    }
}

#[test]
fn a_view_only_session_never_shares_a_pool_key_with_a_normal_one() {
    let scope = Path::new("/srv/agents/teeny");
    assert_eq!(
        pool_key(scope, "thread-1", false),
        "/srv/agents/teeny:thread-1",
        "the normal key is unchanged"
    );
    let view_only = pool_key(scope, "thread-1", true);
    assert_ne!(view_only, pool_key(scope, "thread-1", false));
    assert_ne!(view_only, pool_key(scope, "thread-1:view-only", false));
    assert_ne!(view_only, pool_key(scope, "view-only:thread-1", false));
    assert_ne!(view_only, pool_key(scope, "thread-2", true));
}
