use super::*;
use std::time::Duration;
use tokio::time::Instant;

struct Message {
    text: &'static str,
    media: bool,
}

impl DebounceItem for Message {
    fn chars(&self) -> usize {
        self.text.chars().count()
    }
    fn carries_media(&self) -> bool {
        self.media
    }
    fn mergeable(&self) -> bool {
        !self.text.trim().is_empty()
    }
}

#[test]
fn facade_drives_an_image_and_its_follow_up_into_one_batch() {
    let settings = InboundDebounceConfig::default().settings_for("whatsapp");
    let mut debouncer: KeyedDebouncer<String, Message> = KeyedDebouncer::new();
    let t0 = Instant::now();
    let key = "wa:+15550001".to_string();

    let first = debouncer.push(
        key.clone(),
        Message {
            text: "[IMAGE:photo]",
            media: true,
        },
        &settings,
        t0,
    );
    assert_eq!(first, Push::Opened);
    debouncer.push(
        key.clone(),
        Message {
            text: "what is this?",
            media: false,
        },
        &settings,
        t0 + Duration::from_secs(3),
    );

    assert!(debouncer.poll_ready(t0 + Duration::from_secs(4)).is_empty());
    let ready = debouncer.poll_ready(t0 + Duration::from_secs(7));
    assert_eq!(ready.len(), 1);
    let texts: Vec<_> = ready[0].1.iter().map(|m| m.text).collect();
    assert_eq!(texts, ["[IMAGE:photo]", "what is this?"]);
    assert!(debouncer.is_running(&key));
    debouncer.complete(&key);
    assert!(debouncer.is_empty());
}

#[test]
fn facade_exposes_disabled_settings_and_the_key_limit() {
    let off = InboundDebounceConfig::disabled().settings_for("telegram");
    assert_eq!(off, DebounceSettings::immediate());
    assert_eq!(MAX_TRACKED_KEYS, 2048);
    let over = InboundDebounceOverride::default();
    assert!(over.enabled.is_none());
}
