use super::*;
use crate::core::Outcome;
use crate::voice::factory::{create_stt_provider, create_tts_provider};
use async_trait::async_trait;
use tinyinference_voice::reply::ReplySpeech;

use super::super::traits::SttResult;

struct FakeStt {
    model: String,
}

#[async_trait]
impl SttProvider for FakeStt {
    fn name(&self) -> &'static str {
        "fake-stt"
    }

    async fn transcribe(
        &self,
        _config: &Config,
        _audio_base64: &str,
        _mime_type: Option<&str>,
        _file_name: Option<&str>,
        _language: Option<&str>,
    ) -> Result<Outcome<SttResult>, String> {
        Ok(Outcome::new(
            SttResult {
                text: format!("model={}", self.model),
                provider: "fake-stt".into(),
            },
            vec![],
        ))
    }
}

struct FakeTts {
    voice: Option<String>,
}

#[async_trait]
impl TtsProvider for FakeTts {
    fn name(&self) -> &'static str {
        "fake-tts"
    }

    async fn synthesize(
        &self,
        _config: &Config,
        _text: &str,
        _voice: Option<&str>,
    ) -> Result<Outcome<ReplySpeech>, String> {
        Ok(Outcome::new(
            ReplySpeech {
                audio_base64: self.voice.clone().unwrap_or_default(),
                audio_mime: "audio/mpeg".into(),
                visemes: Vec::new(),
                alignment: None,
            },
            vec![],
        ))
    }
}

#[test]
fn an_unregistered_slug_still_falls_through_to_the_built_in_error() {
    let config = Config::default();
    assert!(create_stt_provider("never-registered", "", &config).is_err());
    assert!(create_tts_provider("never-registered", "", &config).is_err());
}

#[tokio::test]
async fn a_registered_stt_slug_is_built_with_the_effective_model() {
    register_stt_provider("fake-stt", |model, _| {
        Ok(Box::new(FakeStt {
            model: model.to_string(),
        }))
    });
    let config = Config::default();
    let default_model = create_stt_provider("fake-stt", "base", &config).unwrap();
    let suffix_model = create_stt_provider("fake-stt:large", "base", &config).unwrap();
    unregister_stt_provider("fake-stt");

    let text = |p: Box<dyn SttProvider>| async move {
        p.transcribe(&Config::default(), "", None, None, None)
            .await
            .unwrap()
            .value
            .text
    };
    assert_eq!(text(default_model).await, "model=base");
    assert_eq!(text(suffix_model).await, "model=large");
}

#[test]
fn a_registered_slug_answers_under_the_hosted_slug() {
    register_stt_provider("cloud", |_, _| {
        Ok(Box::new(FakeStt {
            model: String::new(),
        }))
    });
    let provider = create_stt_provider("cloud", "", &Config::default()).unwrap();
    unregister_stt_provider("cloud");
    assert_eq!(provider.name(), "fake-stt");
    // With the registration gone the core has no `cloud` provider of its own.
    assert!(create_stt_provider("cloud", "", &Config::default()).is_err());
}

#[tokio::test]
async fn a_registered_tts_provider_gets_the_resolved_voice_and_returns_reply_speech() {
    register_tts_provider("fake-tts", |voice, _| {
        Ok(Box::new(FakeTts {
            voice: voice.map(str::to_string),
        }))
    });
    let config = Config::default();
    let with_voice = create_tts_provider("fake-tts", "alloy", &config).unwrap();
    let without_voice = create_tts_provider("fake-tts", "", &config).unwrap();
    unregister_tts_provider("fake-tts");

    let speech = with_voice
        .synthesize(&config, "hi", None)
        .await
        .unwrap()
        .value;
    assert_eq!(speech.audio_base64, "alloy");
    assert!(speech.visemes.is_empty());
    let speech = without_voice
        .synthesize(&config, "hi", None)
        .await
        .unwrap()
        .value;
    assert_eq!(speech.audio_base64, "");
}
