use super::*;
use crate::openhuman::config::Config;

#[test]
fn removes_write_tools_from_auto_approve() {
    let mut config = Config::default();
    config.autonomy.auto_approve = vec![
        "file_read".into(),
        "file_write".into(),
        "edit_file".into(),
        "glob".into(),
    ];

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.auto_approve_removed, 2);
    assert_eq!(
        config.autonomy.auto_approve,
        vec!["file_read".to_string(), "glob".to_string()]
    );
}

#[test]
fn removes_write_tools_even_when_mixed() {
    let mut config = Config::default();
    config.autonomy.auto_approve = vec!["file_write".into()];

    run(&mut config).expect("migration should succeed");

    assert!(config.autonomy.auto_approve.is_empty());
}
