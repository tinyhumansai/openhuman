use super::*;

#[test]
fn reserved_tool_names_are_detected() {
    for name in RESERVED_TOOL_NAMES {
        assert!(is_reserved_tool_name(name), "{name} must be reserved");
    }
}

#[test]
fn ordinary_tool_names_are_not_reserved() {
    for name in ["file_read", "web_fetch", "task_creates", "memory"] {
        assert!(!is_reserved_tool_name(name), "{name} must not be reserved");
    }
}
