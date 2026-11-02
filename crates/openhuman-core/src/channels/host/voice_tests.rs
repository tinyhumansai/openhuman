use super::*;
use crate::config::schema::ChannelVoiceOverride;
use tinychannels_bus::{ChannelRef, ConversationRef};

fn scope(channel: &str, conversation: &str, turn: &str) -> HostVoiceScope {
    HostVoiceScope {
        configured_instance: channel.into(),
        channel: ChannelRef {
            id: tinychannels_bus::provider_id(channel).into(),
            account_id: Some(channel.into()),
        },
        conversation: ConversationRef {
            id: conversation.into(),
            ..ConversationRef::default()
        },
        turn_id: turn.into(),
    }
}

fn policy_request(
    scope: HostVoiceScope,
    voice: bool,
    explicit: bool,
) -> HostVoiceReplyPolicyRequest {
    HostVoiceReplyPolicyRequest {
        scope,
        inbound_has_voice: voice,
        explicitly_requested: explicit,
    }
}

#[test]
fn policy_mode_ignores_module_supplied_inbound_facts() {
    let cfg = ChannelVoiceConfig::default();
    let voice = resolve_policy(
        &cfg,
        &policy_request(scope("telegram", "c", "t"), true, true),
    );
    let text = resolve_policy(
        &cfg,
        &policy_request(scope("telegram", "c", "t"), false, false),
    );
    assert_eq!(voice.mode, HostVoiceReplyMode::Mirror);
    assert_eq!(
        voice, text,
        "the module applies the facts; the host owns the mode"
    );
}

#[test]
fn policy_mode_follows_thread_then_channel_overrides() {
    let mut cfg = ChannelVoiceConfig::default();
    cfg.channels.insert(
        "telegram".into(),
        ChannelVoiceOverride {
            reply_mode: Some(VoiceReplyMode::Always),
            ..Default::default()
        },
    );
    assert_eq!(
        resolve_policy(
            &cfg,
            &policy_request(scope("telegram", "c", "t"), false, false)
        )
        .mode,
        HostVoiceReplyMode::Always
    );
    cfg.threads.insert("c".into(), VoiceReplyMode::Never);
    assert_eq!(
        resolve_policy(
            &cfg,
            &policy_request(scope("telegram", "c", "t"), false, false)
        )
        .mode,
        HostVoiceReplyMode::Never
    );
    assert_eq!(
        resolve_policy(
            &cfg,
            &policy_request(scope("slack", "other", "t"), false, false)
        )
        .mode,
        HostVoiceReplyMode::Mirror
    );
}

#[test]
fn outbound_length_respects_channel_cap() {
    let mut cfg = ChannelVoiceConfig::default();
    cfg.channels.insert(
        "telegram".into(),
        ChannelVoiceOverride {
            max_outbound_secs: Some(2),
            ..Default::default()
        },
    );
    assert!(check_outbound_length(&cfg, "telegram", &"a".repeat(30)).is_ok());
    assert!(check_outbound_length(&cfg, "telegram", &"a".repeat(31)).is_err());
    assert!(check_outbound_length(&cfg, "slack", &"a".repeat(31)).is_ok());
}

#[tokio::test]
async fn incomplete_scope_is_rejected_before_any_voice_work() {
    let host = ChannelVoiceHost {
        config: Arc::new(Config::default()),
    };
    let bad = HostSpeechSynthesisRequest {
        scope: scope("telegram", "", "t"),
        text: "hello".into(),
        voice: None,
        format: None,
    };
    assert!(host.synthesize_speech(bad).await.is_err());
    let bad = HostVoiceTranscriptionRequest {
        scope: HostVoiceScope::default(),
        ..Default::default()
    };
    assert!(host.transcribe_voice(bad).await.is_err());
}

#[tokio::test]
async fn overlong_reply_is_rejected_without_calling_the_backend() {
    let mut config = Config::default();
    config.channel_voice.max_outbound_secs = 1;
    let host = ChannelVoiceHost {
        config: Arc::new(config),
    };
    let err = host
        .synthesize_speech(HostSpeechSynthesisRequest {
            scope: scope("telegram", "c", "t"),
            text: "x".repeat(100),
            voice: None,
            format: None,
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("maximum spoken duration"));
}

#[cfg(feature = "voice")]
mod stt {
    use super::*;
    use crate::core::Outcome;
    use crate::voice::factory::registry::{register_stt_provider, unregister_stt_provider};
    use crate::voice::factory::{SttProvider, SttResult};

    struct EchoStt {
        fail: bool,
    }

    #[async_trait::async_trait]
    impl SttProvider for EchoStt {
        fn name(&self) -> &'static str {
            "channel-voice-test"
        }

        async fn transcribe(
            &self,
            _config: &Config,
            audio_base64: &str,
            mime_type: Option<&str>,
            file_name: Option<&str>,
            language: Option<&str>,
        ) -> Result<Outcome<SttResult>, String> {
            if self.fail {
                return Err("backend refused the audio".into());
            }
            Ok(Outcome::new(
                SttResult {
                    text: format!(
                        "{audio_base64}|{}|{}|{}",
                        mime_type.unwrap_or("-"),
                        file_name.unwrap_or("-"),
                        language.unwrap_or("-")
                    ),
                    provider: "channel-voice-test".into(),
                },
                vec![],
            ))
        }
    }

    fn host(slug: &str) -> ChannelVoiceHost {
        let mut config = Config::default();
        config.stt_provider = Some(slug.into());
        ChannelVoiceHost {
            config: Arc::new(config),
        }
    }

    #[tokio::test]
    async fn unscoped_transcription_runs_the_configured_provider_with_the_request_hints() {
        register_stt_provider("channel-voice-test-ok", |_, _| {
            Ok(Box::new(EchoStt { fail: false }))
        });
        let result = host("channel-voice-test-ok")
            .transcribe_host(HostTranscriptionRequest {
                scope: None,
                audio_base64: "QUJD".into(),
                mime_type: Some("audio/ogg".into()),
                file_name: Some("note.ogg".into()),
                language: Some("en".into()),
            })
            .await;
        unregister_stt_provider("channel-voice-test-ok");
        let result = result.unwrap();
        assert_eq!(result.text, "QUJD|audio/ogg|note.ogg|en");
        assert_eq!(result.language.as_deref(), Some("en"));
        assert_eq!(result.duration_secs, None);
    }

    #[tokio::test]
    async fn scoped_transcription_validates_the_scope_then_delegates() {
        register_stt_provider("channel-voice-test-scoped", |_, _| {
            Ok(Box::new(EchoStt { fail: false }))
        });
        let host = host("channel-voice-test-scoped");
        let result = host
            .transcribe_voice(HostVoiceTranscriptionRequest {
                scope: scope("telegram", "chat-1", "turn-1"),
                audio_base64: "REVG".into(),
                mime_type: None,
                file_name: None,
                language: None,
            })
            .await;
        unregister_stt_provider("channel-voice-test-scoped");
        assert_eq!(result.unwrap().text, "REVG|-|-|-");
    }

    #[tokio::test]
    async fn provider_failure_and_unknown_provider_surface_as_errors() {
        register_stt_provider("channel-voice-test-fail", |_, _| {
            Ok(Box::new(EchoStt { fail: true }))
        });
        let err = host("channel-voice-test-fail")
            .transcribe_host(HostTranscriptionRequest {
                audio_base64: "QQ==".into(),
                ..Default::default()
            })
            .await
            .unwrap_err();
        unregister_stt_provider("channel-voice-test-fail");
        assert!(
            err.to_string().contains("backend refused the audio"),
            "{err}"
        );

        let err = host("channel-voice-test-never-registered")
            .transcribe_host(HostTranscriptionRequest::default())
            .await;
        assert!(err.is_err());
    }
}

#[tokio::test]
async fn synthesis_fails_cleanly_when_the_voice_module_cannot_be_loaded() {
    let mut config = Config::default();
    config.modules.allow_download = false;
    config.modules.source = crate::config::schema::ModuleSourceMode::Release;
    let host = ChannelVoiceHost {
        config: Arc::new(config),
    };
    let err = host
        .synthesize_speech(HostSpeechSynthesisRequest {
            scope: scope("telegram", "c", "t"),
            text: "short reply".into(),
            voice: Some("alloy".into()),
            format: None,
        })
        .await;
    assert!(err.is_err(), "no voice module is available offline");
}

#[test]
fn reply_policy_reports_the_configured_mode_for_the_scoped_turn() {
    let mut config = Config::default();
    config
        .channel_voice
        .threads
        .insert("chat-9".into(), VoiceReplyMode::SpeakReply);
    let host = ChannelVoiceHost {
        config: Arc::new(config),
    };
    let policy = host.reply_policy(&policy_request(
        scope("telegram", "chat-9", "t"),
        true,
        false,
    ));
    assert_eq!(policy.mode, HostVoiceReplyMode::SpeakReply);
    let default = host.reply_policy(&policy_request(
        scope("telegram", "other", "t"),
        false,
        true,
    ));
    assert_eq!(default.mode, HostVoiceReplyMode::Mirror);
}
