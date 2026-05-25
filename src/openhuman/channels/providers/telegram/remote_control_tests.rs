use super::*;

#[test]
fn parse_remote_commands() {
    assert_eq!(
        parse_telegram_remote_command("/status"),
        Some(TelegramRemoteCommand::Status)
    );
    assert_eq!(
        parse_telegram_remote_command("/status@MyBot"),
        Some(TelegramRemoteCommand::Status)
    );
    assert_eq!(
        parse_telegram_remote_command("  /sessions  "),
        Some(TelegramRemoteCommand::Sessions)
    );
    assert_eq!(
        parse_telegram_remote_command("/new"),
        Some(TelegramRemoteCommand::New)
    );
    assert_eq!(
        parse_telegram_remote_command("/help"),
        Some(TelegramRemoteCommand::Help)
    );
    assert_eq!(
        parse_telegram_remote_command(" /STATUS@OpenHumanBot now "),
        Some(TelegramRemoteCommand::Status)
    );
    // Case insensitivity for other variants
    assert_eq!(
        parse_telegram_remote_command("/Sessions"),
        Some(TelegramRemoteCommand::Sessions)
    );
    assert_eq!(
        parse_telegram_remote_command("/NEW@Bot"),
        Some(TelegramRemoteCommand::New)
    );
    assert!(parse_telegram_remote_command("hello").is_none());
    assert!(parse_telegram_remote_command("/model").is_none());
    assert!(parse_telegram_remote_command("/unknown@OpenHumanBot").is_none());
}
