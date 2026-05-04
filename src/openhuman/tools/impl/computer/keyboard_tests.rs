use super::*;

fn make_tool() -> KeyboardTool {
    KeyboardTool::new(Arc::new(SecurityPolicy::default()))
}

#[test]
fn schema_has_required_action() {
    let tool = make_tool();
    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["action"]));
}

#[test]
fn schema_enumerates_actions() {
    let tool = make_tool();
    let schema = tool.parameters_schema();
    let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
    let names: Vec<&str> = actions.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(names.contains(&"type"));
    assert!(names.contains(&"press"));
    assert!(names.contains(&"hotkey"));
}

#[test]
fn permission_is_dangerous() {
    assert_eq!(make_tool().permission_level(), PermissionLevel::Dangerous);
}

#[test]
fn name_is_keyboard() {
    assert_eq!(make_tool().name(), "keyboard");
}

// ── parse_key tests ──────────────────────────────────────────

#[test]
fn parse_key_modifiers() {
    assert_eq!(parse_key("Ctrl"), Some(Key::Control));
    assert_eq!(parse_key("control"), Some(Key::Control));
    assert_eq!(parse_key("Shift"), Some(Key::Shift));
    assert_eq!(parse_key("Alt"), Some(Key::Alt));
    assert_eq!(parse_key("Option"), Some(Key::Alt));
    assert_eq!(parse_key("Cmd"), Some(Key::Meta));
    assert_eq!(parse_key("Command"), Some(Key::Meta));
    assert_eq!(parse_key("Meta"), Some(Key::Meta));
    assert_eq!(parse_key("Super"), Some(Key::Meta));
    assert_eq!(parse_key("Win"), Some(Key::Meta));
}

#[test]
fn parse_key_navigation() {
    assert_eq!(parse_key("Enter"), Some(Key::Return));
    assert_eq!(parse_key("Return"), Some(Key::Return));
    assert_eq!(parse_key("Tab"), Some(Key::Tab));
    assert_eq!(parse_key("Escape"), Some(Key::Escape));
    assert_eq!(parse_key("Esc"), Some(Key::Escape));
    assert_eq!(parse_key("Backspace"), Some(Key::Backspace));
    assert_eq!(parse_key("Delete"), Some(Key::Delete));
    assert_eq!(parse_key("Space"), Some(Key::Space));
}

#[test]
fn parse_key_arrows() {
    assert_eq!(parse_key("Up"), Some(Key::UpArrow));
    assert_eq!(parse_key("Down"), Some(Key::DownArrow));
    assert_eq!(parse_key("Left"), Some(Key::LeftArrow));
    assert_eq!(parse_key("Right"), Some(Key::RightArrow));
}

#[test]
fn parse_key_function_keys() {
    assert_eq!(parse_key("F1"), Some(Key::F1));
    assert_eq!(parse_key("f5"), Some(Key::F5));
    assert_eq!(parse_key("F12"), Some(Key::F12));
}

#[test]
fn parse_key_single_chars() {
    assert_eq!(parse_key("a"), Some(Key::Unicode('a')));
    assert_eq!(parse_key("A"), Some(Key::Unicode('A')));
    assert_eq!(parse_key("5"), Some(Key::Unicode('5')));
    assert_eq!(parse_key("/"), Some(Key::Unicode('/')));
}

#[test]
fn parse_key_unknown_returns_none() {
    assert_eq!(parse_key("FooBar"), None);
    assert_eq!(parse_key(""), None);
}

#[test]
fn modifier_detection() {
    assert!(is_modifier(&Key::Control));
    assert!(is_modifier(&Key::Shift));
    assert!(is_modifier(&Key::Alt));
    assert!(is_modifier(&Key::Meta));
    assert!(!is_modifier(&Key::Return));
    assert!(!is_modifier(&Key::Unicode('a')));
}

// ── execute validation tests ─────────────────────────────────

#[tokio::test]
async fn missing_action_returns_error() {
    let tool = make_tool();
    let result = tool.execute(json!({})).await;
    assert!(result.is_err() || result.unwrap().is_error);
}

#[tokio::test]
async fn unknown_action_returns_error() {
    let tool = make_tool();
    let result = tool.execute(json!({"action": "smash"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Unknown keyboard action"));
}

#[tokio::test]
async fn type_missing_text_returns_error() {
    let tool = make_tool();
    let result = tool.execute(json!({"action": "type"})).await;
    assert!(result.is_err() || result.unwrap().is_error);
}

#[tokio::test]
async fn type_empty_text_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "type", "text": ""}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn press_missing_key_returns_error() {
    let tool = make_tool();
    let result = tool.execute(json!({"action": "press"})).await;
    assert!(result.is_err() || result.unwrap().is_error);
}

#[tokio::test]
async fn press_unknown_key_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "press", "key": "FooBarBaz"}))
        .await;
    assert!(result.is_err() || result.unwrap().is_error);
}

#[tokio::test]
async fn hotkey_missing_keys_returns_error() {
    let tool = make_tool();
    let result = tool.execute(json!({"action": "hotkey"})).await;
    assert!(result.is_err() || result.unwrap().is_error);
}

#[tokio::test]
async fn hotkey_empty_array_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "hotkey", "keys": []}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn hotkey_too_many_keys_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "hotkey", "keys": ["a","b","c","d","e","f","g"]}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn type_too_long_returns_error() {
    let tool = make_tool();
    let long_text = "x".repeat(MAX_TYPE_LENGTH + 1);
    let result = tool
        .execute(json!({"action": "type", "text": long_text}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("too long"));
}

// ── hotkey validation tests ──────────────────────────────────

#[tokio::test]
async fn hotkey_non_string_entry_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "hotkey", "keys": ["Ctrl", 1]}))
        .await;
    assert!(result.is_err() || result.unwrap().is_error);
}

#[tokio::test]
async fn hotkey_modifier_only_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "hotkey", "keys": ["Ctrl"]}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn hotkey_non_modifier_before_last_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "hotkey", "keys": ["a", "Ctrl"]}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn hotkey_modifier_as_last_returns_error() {
    let tool = make_tool();
    let result = tool
        .execute(json!({"action": "hotkey", "keys": ["Ctrl", "Shift"]}))
        .await
        .unwrap();
    assert!(result.is_error);
}
