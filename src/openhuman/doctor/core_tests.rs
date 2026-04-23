use super::*;

#[test]
fn config_validation_warns_no_channels() {
    let config = Config::default();
    let mut items = vec![];
    check_config_semantics(&config, &mut items);
    let ch_item = items.iter().find(|i| i.message.contains("channel"));
    assert!(ch_item.is_some());
    assert_eq!(ch_item.unwrap().severity, Severity::Warn);
}

#[test]
fn truncate_for_display_short() {
    let s = "hello";
    assert_eq!(truncate_for_display(s, 10), s);
}

#[test]
fn truncate_for_display_long() {
    let s = "abcdefghijklmnopqrstuvwxyz";
    let truncated = truncate_for_display(s, 5);
    assert!(truncated.starts_with("abcde"));
    assert!(truncated.ends_with("..."));
}
