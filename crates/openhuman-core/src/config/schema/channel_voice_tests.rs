use super::*;

fn config() -> ChannelVoiceConfig {
    let mut cfg = ChannelVoiceConfig::default();
    cfg.channels.insert(
        "telegram".into(),
        ChannelVoiceOverride {
            reply_mode: Some(VoiceReplyMode::Always),
            voice: Some("nova".into()),
            max_outbound_secs: Some(30),
        },
    );
    cfg.threads.insert("t-1".into(), VoiceReplyMode::Never);
    cfg
}

#[test]
fn default_mode_is_mirror() {
    let cfg = ChannelVoiceConfig::default();
    assert_eq!(cfg.resolve_mode("slack", "x"), VoiceReplyMode::Mirror);
    assert_eq!(cfg.max_outbound_secs, 120);
}

#[test]
fn channel_override_beats_global() {
    assert_eq!(
        config().resolve_mode("telegram", "other"),
        VoiceReplyMode::Always
    );
}

#[test]
fn thread_override_beats_channel() {
    assert_eq!(
        config().resolve_mode("telegram", "t-1"),
        VoiceReplyMode::Never
    );
}

#[test]
fn voice_and_cap_fall_back_to_global() {
    let mut cfg = config();
    cfg.voice = Some("alloy".into());
    assert_eq!(cfg.resolve_voice("telegram"), Some("nova"));
    assert_eq!(cfg.resolve_voice("slack"), Some("alloy"));
    assert_eq!(cfg.resolve_max_outbound_secs("telegram"), 30);
    assert_eq!(cfg.resolve_max_outbound_secs("slack"), 120);
}

#[test]
fn parses_from_toml() {
    let cfg: ChannelVoiceConfig = toml::from_str(
        "reply_mode = \"never\"\n[channels.telegram]\nreply_mode = \"speak_reply\"\n[threads]\nabc = \"always\"\n",
    )
    .unwrap();
    assert_eq!(cfg.resolve_mode("slack", "z"), VoiceReplyMode::Never);
    assert_eq!(
        cfg.resolve_mode("telegram", "z"),
        VoiceReplyMode::SpeakReply
    );
    assert_eq!(cfg.resolve_mode("telegram", "abc"), VoiceReplyMode::Always);
}
