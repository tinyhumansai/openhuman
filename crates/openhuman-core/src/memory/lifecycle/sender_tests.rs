use super::*;

fn origin(channel: &str, sender: Option<&str>, name: Option<&str>) -> AgentTurnOrigin {
    AgentTurnOrigin::ExternalChannel {
        channel: channel.into(),
        sender: sender.map(str::to_string),
        sender_name: name.map(str::to_string),
        reply_target: "chat".into(),
        message_id: "m-1".into(),
        history_key: Some("k".into()),
        channel_session: None,
    }
}

fn id(channel: &str, sender: &str) -> Option<String> {
    channel_actor(&origin(channel, Some(sender), None)).map(|actor| actor.id)
}

#[test]
fn a_phone_sender_is_its_dial_digits_with_the_name() {
    assert_eq!(
        channel_actor(&origin("whatsapp", Some("+15551234567"), Some(" Mum "))),
        Some(ObservedActor {
            id: "user:+15551234567".into(),
            name: Some("Mum".into()),
        })
    );
    assert_eq!(
        id("signal", "+1 (555) 123-4567").as_deref(),
        Some("user:+15551234567")
    );
    assert_eq!(id("SMS", "5551234567").as_deref(), Some("user:5551234567"));
}

#[test]
fn an_email_sender_is_lower_cased() {
    assert_eq!(
        id("imessage", "Priya@Acme.com").as_deref(),
        Some("user:priya@acme.com")
    );
    assert_eq!(
        id("email", "priya@acme.com").as_deref(),
        Some("user:priya@acme.com")
    );
}

#[test]
fn any_other_sender_is_named_by_its_channel() {
    assert_eq!(
        id("telegram", "123456789").as_deref(),
        Some("telegram:123456789"),
        "a Telegram id is digits, not a phone number"
    );
    assert_eq!(id("telegram", "@priya").as_deref(), Some("telegram:@priya"));
    assert_eq!(
        id("discord", "112233445566778899").as_deref(),
        Some("discord:112233445566778899")
    );
    assert_eq!(
        id("whatsapp", "98765432101234@lid").as_deref(),
        Some("whatsapp:98765432101234@lid"),
        "a WhatsApp LID is not an email address"
    );
}

#[test]
fn nothing_is_attributed_without_a_channel_sender() {
    assert_eq!(channel_actor(&origin("whatsapp", None, Some("Mum"))), None);
    assert_eq!(id("whatsapp", "  "), None);
    assert_eq!(id("slack", "two words"), None);
    assert_eq!(id(" ", "+15551234567"), None);
    assert_eq!(channel_actor(&AgentTurnOrigin::Cli), None);
    assert_eq!(channel_actor(&AgentTurnOrigin::DirectChat), None);

    let cron = AgentTurnOrigin::ExternalChannel {
        channel: "telegram".into(),
        sender: Some("alice".into()),
        sender_name: None,
        reply_target: "42".into(),
        message_id: "cron:job-1:run-1".into(),
        history_key: Some("k".into()),
        channel_session: None,
    };
    assert_eq!(channel_actor(&cron), None, "a cron run is the job's prompt");
}
