use super::*;

#[cfg(feature = "whatsapp-web")]
fn make_channel() -> WhatsAppWebChannel {
    WhatsAppWebChannel::new(
        "/tmp/test-whatsapp.db".into(),
        None,
        None,
        vec!["+1234567890".into()],
    )
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_channel_name() {
    let ch = make_channel();
    assert_eq!(ch.name(), "whatsapp");
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_number_allowed_exact() {
    let ch = make_channel();
    assert!(ch.is_number_allowed("+1234567890"));
    assert!(!ch.is_number_allowed("+9876543210"));
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_number_allowed_wildcard() {
    let ch = WhatsAppWebChannel::new("/tmp/test.db".into(), None, None, vec!["*".into()]);
    assert!(ch.is_number_allowed("+1234567890"));
    assert!(ch.is_number_allowed("+9999999999"));
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_number_denied_empty() {
    let ch = WhatsAppWebChannel::new("/tmp/test.db".into(), None, None, vec![]);
    // Empty allowed_numbers means "allow all" (same behavior as Cloud API)
    assert!(ch.is_number_allowed("+1234567890"));
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_normalize_phone_adds_plus() {
    let ch = make_channel();
    assert_eq!(ch.normalize_phone("1234567890"), "+1234567890");
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_normalize_phone_preserves_plus() {
    let ch = make_channel();
    assert_eq!(ch.normalize_phone("+1234567890"), "+1234567890");
}

#[tokio::test]
#[cfg(feature = "whatsapp-web")]
async fn whatsapp_web_health_check_disconnected() {
    let ch = make_channel();
    assert!(!ch.health_check().await);
}

#[tokio::test]
#[cfg(feature = "whatsapp-web")]
async fn whatsapp_web_health_check_tracks_connected_flag() {
    let ch = make_channel();
    assert!(!ch.health_check().await);
    ch.connected.store(true, Ordering::Release);
    assert!(ch.health_check().await);
    ch.connected.store(false, Ordering::Release);
    assert!(!ch.health_check().await);
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_compute_reply_target_dm_pn() {
    assert_eq!(
        WhatsAppWebChannel::compute_reply_target("123@s.whatsapp.net", "+1234567890"),
        "+1234567890"
    );
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_compute_reply_target_dm_lid() {
    assert_eq!(
        WhatsAppWebChannel::compute_reply_target("abc@lid", "+1234567890"),
        "+1234567890"
    );
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_compute_reply_target_group() {
    assert_eq!(
        WhatsAppWebChannel::compute_reply_target("987654@g.us", "+1234567890"),
        "987654@g.us"
    );
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_redact_phone_e164() {
    assert_eq!(WhatsAppWebChannel::redact_phone("+1234567890"), "+***7890");
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_redact_phone_no_plus() {
    assert_eq!(WhatsAppWebChannel::redact_phone("1234567890"), "***7890");
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_redact_phone_short_input() {
    // Pathological short inputs collapse to a generic mask rather than
    // exposing the entire identifier.
    assert_eq!(WhatsAppWebChannel::redact_phone("+12"), "+****");
    assert_eq!(WhatsAppWebChannel::redact_phone("12"), "****");
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_extract_message_text_prefers_conversation() {
    assert_eq!(
        WhatsAppWebChannel::extract_message_text(Some("hello"), Some("ignored")),
        "hello"
    );
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_extract_message_text_falls_back_to_extended() {
    assert_eq!(
        WhatsAppWebChannel::extract_message_text(None, Some("from extended")),
        "from extended"
    );
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_extract_message_text_empty_when_missing() {
    assert_eq!(WhatsAppWebChannel::extract_message_text(None, None), "");
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_is_group_jid_recognises_group() {
    assert!(WhatsAppWebChannel::is_group_jid("123456@g.us"));
    assert!(WhatsAppWebChannel::is_group_jid("  4567@g.us  "));
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_is_group_jid_rejects_non_group() {
    assert!(!WhatsAppWebChannel::is_group_jid("+1234567890"));
    assert!(!WhatsAppWebChannel::is_group_jid("123@s.whatsapp.net"));
    assert!(!WhatsAppWebChannel::is_group_jid("abc@lid"));
    assert!(!WhatsAppWebChannel::is_group_jid(""));
}

/// Regression for CodeRabbit finding: an `@g.us` reply target was being
/// silently dropped because the outbound path normalised the JID to
/// `+<group-id>` and missed the per-number allowlist. After provenance
/// is recorded, an allowed user replying back into the group they came
/// from must succeed.
#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_should_allow_outbound_provenanced_group_allowed() {
    let ch = make_channel(); // allowed_numbers = ["+1234567890"]
    ch.allowed_groups
        .lock()
        .insert("987654321@g.us".to_string());
    assert!(ch.should_allow_outbound("987654321@g.us"));
}

/// Regression for the follow-up CodeRabbit finding: a blanket `@g.us`
/// bypass is itself a vulnerability — a caller able to set `recipient`
/// could post into arbitrary joined groups. Groups without recorded
/// provenance must stay blocked.
#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_should_allow_outbound_unrelated_group_blocked() {
    let ch = make_channel();
    ch.allowed_groups
        .lock()
        .insert("987654321@g.us".to_string());
    assert!(!ch.should_allow_outbound("11111@g.us"));
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_should_allow_outbound_group_without_provenance_blocked() {
    let ch = make_channel();
    // empty allowed_groups
    assert!(!ch.should_allow_outbound("987654321@g.us"));
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_redact_recipient_pn_jid() {
    assert_eq!(
        WhatsAppWebChannel::redact_recipient("1234567890@s.whatsapp.net"),
        "***7890@s.whatsapp.net"
    );
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_redact_recipient_group_jid() {
    assert_eq!(
        WhatsAppWebChannel::redact_recipient("987654321@g.us"),
        "***4321@g.us"
    );
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_redact_recipient_bare_phone() {
    assert_eq!(
        WhatsAppWebChannel::redact_recipient("+1234567890"),
        "+***7890"
    );
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_should_allow_outbound_dm_blocks_unallowed() {
    let ch = make_channel();
    assert!(!ch.should_allow_outbound("+9999999999"));
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_should_allow_outbound_dm_allows_match() {
    let ch = make_channel();
    assert!(ch.should_allow_outbound("+1234567890"));
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_should_allow_outbound_wildcard_passes_dm() {
    let ch = WhatsAppWebChannel::new("/tmp/t.db".into(), None, None, vec!["*".into()]);
    assert!(ch.should_allow_outbound("+9999999999"));
}

#[test]
#[cfg(feature = "whatsapp-web")]
fn whatsapp_web_should_allow_outbound_empty_allowlist_passes_dm() {
    let ch = WhatsAppWebChannel::new("/tmp/t.db".into(), None, None, vec![]);
    assert!(ch.should_allow_outbound("+9999999999"));
}
