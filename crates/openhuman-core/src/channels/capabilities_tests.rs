use super::*;

#[test]
fn telegram_alias_ignores_thread_in_history_key() {
    assert!(capabilities_for("tg:123").history_key_ignores_thread);
    assert!(capabilities_for("telegram").chat_approvals);
}

#[test]
fn chat_providers_support_approvals_and_remote_control() {
    for provider in ["discord", "slack", "signal", "whatsapp_web", "yuanbao"] {
        let caps = capabilities_for(provider);
        assert!(caps.chat_approvals && caps.remote_control, "{provider}");
        assert!(!caps.history_key_ignores_thread, "{provider}");
    }
}

#[test]
fn unknown_and_oneway_providers_have_no_capabilities() {
    for provider in ["email", "webhook", "cli", "brand-new"] {
        assert_eq!(
            capabilities_for(provider),
            HostChannelCapabilities::default()
        );
    }
}
