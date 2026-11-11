use super::*;

#[test]
fn the_setting_off_keeps_the_human_grant_whatever_the_keyring() {
    for backend in [
        BackendKind::Host,
        BackendKind::Memory,
        BackendKind::File,
        BackendKind::EncryptedFile,
    ] {
        for operator_key in [false, true] {
            assert_eq!(
                unlock_mode(false, false, operator_key, backend),
                UnlockMode::Interactive,
                "{backend:?} operator_key={operator_key}"
            );
        }
    }
}

#[test]
fn an_operator_key_on_a_file_keyring_unlocks_unattended() {
    assert_eq!(
        unlock_mode(true, false, true, BackendKind::File),
        UnlockMode::Unattended
    );
    assert_eq!(
        unlock_mode(true, false, true, BackendKind::EncryptedFile),
        UnlockMode::Unattended
    );
}

#[test]
fn the_setting_is_ignored_without_an_operator_supplied_file_key() {
    // A key the app minted or keeps in the OS keychain is the desktop's: a
    // person still answers.
    assert_eq!(
        unlock_mode(true, false, false, BackendKind::EncryptedFile),
        UnlockMode::IgnoredWithoutOperatorKey
    );
    assert_eq!(
        unlock_mode(true, false, false, BackendKind::File),
        UnlockMode::IgnoredWithoutOperatorKey
    );
    assert_eq!(
        unlock_mode(true, false, true, BackendKind::Host),
        UnlockMode::IgnoredWithoutOperatorKey
    );
    assert_eq!(
        unlock_mode(true, false, true, BackendKind::Memory),
        UnlockMode::IgnoredWithoutOperatorKey
    );
}

#[test]
fn a_saas_core_ignores_the_setting() {
    for backend in [BackendKind::File, BackendKind::EncryptedFile] {
        assert_eq!(
            unlock_mode(true, true, true, backend),
            UnlockMode::IgnoredOnSaas
        );
    }
    assert_eq!(
        unlock_mode(false, true, true, BackendKind::File),
        UnlockMode::Interactive
    );
}

#[test]
fn a_default_config_is_interactive() {
    let _assumed = assume_operator_key();
    assert_eq!(mode_for(&Config::default()), UnlockMode::Interactive);
    let mut config = Config::default();
    config.secrets.unattended_unlock = true;
    assert_eq!(mode_for(&config), UnlockMode::Unattended);
}

fn parked(tool: &str, tool_call_id: Option<&str>) -> crate::security::approval::PendingApproval {
    let mut pending = crate::security::approval::PendingApproval::new(
        "req-1",
        tool,
        "Unlock configuration secrets",
        serde_json::json!({}),
        None,
    );
    pending.tool_call_id = tool_call_id.map(str::to_owned);
    pending
}

#[test]
fn only_the_park_this_call_raised_is_answered() {
    let tool = "native_secret_grant";
    assert!(is_our_park(
        &parked(tool, Some("corr-ours")),
        tool,
        "corr-ours"
    ));
    assert!(
        !is_our_park(&parked(tool, Some("corr-theirs")), tool, "corr-ours"),
        "another caller's grant"
    );
    assert!(
        !is_our_park(&parked(tool, None), tool, "corr-ours"),
        "an uncorrelated grant (a person's desktop prompt)"
    );
    assert!(
        !is_our_park(&parked("shell", Some("corr-ours")), tool, "corr-ours"),
        "another tool"
    );
}
