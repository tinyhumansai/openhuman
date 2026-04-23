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
