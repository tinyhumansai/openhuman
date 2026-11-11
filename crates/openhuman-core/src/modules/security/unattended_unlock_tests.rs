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
                unlock_mode(false, operator_key, backend),
                UnlockMode::Interactive,
                "{backend:?} operator_key={operator_key}"
            );
        }
    }
}

#[test]
fn an_operator_key_on_a_file_keyring_unlocks_unattended() {
    assert_eq!(
        unlock_mode(true, true, BackendKind::File),
        UnlockMode::Unattended
    );
    assert_eq!(
        unlock_mode(true, true, BackendKind::EncryptedFile),
        UnlockMode::Unattended
    );
}

#[test]
fn the_setting_is_ignored_without_an_operator_supplied_file_key() {
    // A key the app minted or keeps in the OS keychain is the desktop's: a
    // person still answers.
    assert_eq!(
        unlock_mode(true, false, BackendKind::EncryptedFile),
        UnlockMode::IgnoredWithoutOperatorKey
    );
    assert_eq!(
        unlock_mode(true, false, BackendKind::File),
        UnlockMode::IgnoredWithoutOperatorKey
    );
    assert_eq!(
        unlock_mode(true, true, BackendKind::Host),
        UnlockMode::IgnoredWithoutOperatorKey
    );
    assert_eq!(
        unlock_mode(true, true, BackendKind::Memory),
        UnlockMode::IgnoredWithoutOperatorKey
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
