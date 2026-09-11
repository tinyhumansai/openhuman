//! Tests for the voice call client.
//!
//! Nothing here loads a module. What is testable without one is the part
//! that decides what a caller does next: the intent wire contract, the
//! mode spellings, and that the unavailable path is reached without a
//! broker. The round trips themselves are covered where they can be
//! honest — `tinyvoice`'s own loader E2E, which drives a real module over
//! a real broker against the published artifact.

use super::{
    clamped, encode_samples, hallucination_mode_wire, HallucinationMode, VoiceCallError,
    VoiceIntent,
};
use crate::openhuman::config::Config;
/// The intent tags are a wire contract with the module. A rename on either
/// side turns a real command into `Unknown`, which degrades silently — the
/// user's "pause" simply goes to the agent instead — so every tag is
/// pinned here rather than trusted to match by inspection.
#[test]
fn every_intent_tag_decodes() {
    let cases: &[(&str, VoiceIntent)] = &[
        (r#"{"intent":"pause"}"#, VoiceIntent::Pause),
        (r#"{"intent":"resume"}"#, VoiceIntent::Resume),
        (r#"{"intent":"next"}"#, VoiceIntent::Next),
        (r#"{"intent":"previous"}"#, VoiceIntent::Previous),
        (r#"{"intent":"volume_up"}"#, VoiceIntent::VolumeUp),
        (r#"{"intent":"volume_down"}"#, VoiceIntent::VolumeDown),
        (r#"{"intent":"mute"}"#, VoiceIntent::Mute),
        (r#"{"intent":"unmute"}"#, VoiceIntent::Unmute),
        (r#"{"intent":"unknown"}"#, VoiceIntent::Unknown),
        (
            r#"{"intent":"set_volume","percent":40}"#,
            VoiceIntent::SetVolume { percent: 40 },
        ),
        (
            r#"{"intent":"play","query":"numb"}"#,
            VoiceIntent::Play {
                query: "numb".to_string(),
            },
        ),
        (
            r#"{"intent":"open_app","app":"slack"}"#,
            VoiceIntent::OpenApp {
                app: "slack".to_string(),
            },
        ),
    ];
    for (json, expected) in cases {
        let decoded: VoiceIntent = serde_json::from_str(json).expect(json);
        assert_eq!(&decoded, expected, "decoding {json}");
    }
}

#[test]
fn an_unrecognised_tag_degrades_to_unknown_rather_than_failing() {
    // A module newer than this host may name an intent we have never heard
    // of. Deferring to the agent is the correct handling; a decode error
    // would turn a forward-compatible addition into a broken call.
    let decoded: VoiceIntent =
        serde_json::from_str(r#"{"intent":"summon_helicopter"}"#).expect("decodes");
    assert_eq!(decoded, VoiceIntent::Unknown);
}

#[test]
fn hallucination_modes_use_the_wire_spelling() {
    // The module rejects an unknown mode rather than defaulting, so a typo
    // here is a hard failure at runtime rather than a silent mode swap.
    assert_eq!(
        hallucination_mode_wire(HallucinationMode::Dictation),
        "dictation"
    );
    assert_eq!(
        hallucination_mode_wire(HallucinationMode::Conversation),
        "conversation"
    );
}

#[test]
fn samples_encode_as_little_endian_f32() {
    use base64::Engine as _;
    let encoded = encode_samples(&[1.0f32, -1.0f32]);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&encoded)
        .expect("valid base64");
    assert_eq!(bytes.len(), 8);
    assert_eq!(&bytes[0..4], &1.0f32.to_le_bytes());
    assert_eq!(&bytes[4..8], &(-1.0f32).to_le_bytes());
}

#[test]
fn errors_render_as_their_message() {
    assert_eq!(
        VoiceCallError::Unavailable("downloads are off".to_string()).to_string(),
        "downloads are off"
    );
    assert_eq!(
        VoiceCallError::Failed("bad payload".to_string()).to_string(),
        "bad payload"
    );
}

/// A config with modules enabled but nothing fetchable.
fn offline_config() -> Config {
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config
}

#[tokio::test]
async fn a_disabled_host_reports_unavailable_without_starting_a_broker() {
    // Every entry point has to reach the unavailable path on its own: they
    // each call `ensure_loaded` separately, so one of them forgetting to
    // would only show up as a hang or a panic in the field.
    let mut config = offline_config();
    config.modules.enabled = false;

    assert!(matches!(
        super::route(&config, "pause").await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::extract_command(&config, "hey tiny pause", "Hey Tiny").await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::wake_word_present(&config, "hey tiny", "Hey Tiny").await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::is_hallucinated(&config, "okay", HallucinationMode::Dictation).await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::encode_wav(&config, &[0.1, 0.2], 16_000).await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::prepare_capture(&config, &[0.1, 0.2], 32_000, 2, 0.0).await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::ensure_ready(&config).await,
        Err(VoiceCallError::Unavailable(_))
    ));
}

#[test]
fn the_registry_entry_matches_the_interface_this_client_calls() {
    // The registry is a plain `const` table and cannot name a gated crate, so
    // the bus name and object path are still written out there by hand. This
    // is what checks them against the contract's own constants — a mismatch is
    // not a compile error, it is a `NameHasNoOwner` at first use, in the field,
    // on whichever platform nobody tested.
    let record =
        crate::openhuman::modules::registry::find("tinyvoice").expect("tinyvoice is registered");
    assert_eq!(record.bus_name, tinyvoice_bus::names::BUS_NAME);
    assert_eq!(record.object_path, tinyvoice_bus::names::OBJECT_PATH);
    assert!(
        record.object_path.starts_with('/') && !record.object_path.contains('.'),
        "an object path with a dot in it is rejected by the loader, not by the compiler"
    );
}

/// Drive the real published module through OpenHuman's own module host.
///
/// `#[ignore]`d, and it must stay that way. The bus belongs to whichever
/// runtime creates it, so two `#[tokio::test]`s that each load a module find
/// the second one's broker already dead and hang rather than fail. A
/// module-backed test has to be the only one in its process:
///
/// ```sh
/// GGML_NATIVE=OFF cargo test --lib --features "$(bash scripts/ci/product-features.sh)" \
///   --ignored --exact --nocapture \
///   openhuman::modules::voice::tests::the_published_module_answers_through_this_client
/// ```
///
/// It also downloads from the pinned release, so it needs network and is not
/// something CI should carry.
#[tokio::test]
#[ignore = "loads a real module: needs network, and must be alone in its process"]
async fn the_published_module_answers_through_this_client() {
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = true;

    // The whole client surface, against the artifact the registry pins.
    assert_eq!(
        super::route(&config, "please pause the music")
            .await
            .expect("route"),
        VoiceIntent::Pause
    );
    assert_eq!(
        super::route(&config, "set volume to 40")
            .await
            .expect("route"),
        VoiceIntent::SetVolume { percent: 40 }
    );
    assert_eq!(
        super::route(&config, "what is the weather")
            .await
            .expect("route"),
        VoiceIntent::Unknown
    );
    assert_eq!(
        super::extract_command(&config, "hey tiny open slack", "Hey Tiny")
            .await
            .expect("extract"),
        Some("open slack".to_string())
    );
    assert_eq!(
        super::extract_command(&config, "open slack", "Hey Tiny")
            .await
            .expect("extract"),
        None
    );
    assert!(super::wake_word_present(&config, "hey tiny", "Hey Tiny")
        .await
        .expect("present"));

    // The mode split is the reason this call takes a mode at all.
    assert!(
        super::is_hallucinated(&config, "okay", HallucinationMode::Dictation)
            .await
            .expect("dictation")
    );
    assert!(
        !super::is_hallucinated(&config, "okay", HallucinationMode::Conversation)
            .await
            .expect("conversation")
    );

    // 400 interleaved stereo samples at 32 kHz -> 100 mono at 16 kHz.
    let stereo: Vec<f32> = (0..400).map(|i| ((i as f32) / 20.0).sin() * 0.5).collect();
    let wav = super::prepare_capture(&config, &stereo, 32_000, 2, 0.0)
        .await
        .expect("prepare_capture");
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(wav.len(), 44 + 100 * 2);
    assert_eq!(
        u32::from_le_bytes(wav[24..28].try_into().expect("4 bytes")),
        16_000,
        "the header must declare the rate the samples were converted to"
    );

    // The capture-side pipeline the always-on loop now runs entirely remotely.
    let mono = super::prepare_frames(&config, &stereo, 32_000, 2)
        .await
        .expect("prepare_frames");
    assert_eq!(mono.len(), 100, "samples, not a container");

    // 100 samples with a 320-sample frame is ONE short frame, not zero: the
    // module slices with `chunks`, not `chunks_exact`, so the trailing partial
    // frame is measured rather than dropped. Dropping it would lose the end of
    // an utterance.
    let energies = super::frame_energies(&config, &mono, 320)
        .await
        .expect("frame_energies");
    assert_eq!(energies.len(), 1, "a short trailing frame still counts");

    // And a frame size that divides evenly gives the expected count.
    let energies = super::frame_energies(&config, &mono, 25)
        .await
        .expect("frame_energies");
    assert_eq!(energies.len(), 4);

    // PCM16 framing must return the caller's samples untouched.
    let pcm: Vec<i16> = vec![0, 1, -1, i16::MAX];
    let wav = super::encode_wav_pcm16(&config, &pcm, 16_000, 1)
        .await
        .expect("encode_wav_pcm16");
    let expected: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    assert_eq!(&wav[44..], &expected[..]);

    // A VAD session must segment across separate pushes — the whole reason it
    // is a session and not a batch.
    let vad = super::VadConfig {
        onset_threshold: 0.1,
        hangover_ms: 100,
        min_speech_ms: 60,
        max_utterance_ms: 5_000,
    };
    let session = super::VadSession::open(&config, vad)
        .await
        .expect("VadOpen");

    let events = session
        .push(&config, 20, &[0.5f32; 6])
        .await
        .expect("VadPush");
    assert!(
        matches!(
            events.as_slice(),
            [super::IndexedVadEvent {
                event: super::VadEvent::SpeechStart,
                ..
            }]
        ),
        "expected a single speech start, got {events:?}"
    );
    assert!(session.is_speaking(&config).await.expect("is_speaking"));

    let events = session
        .push(&config, 20, &[0.0f32; 6])
        .await
        .expect("VadPush");
    match events.as_slice() {
        [super::IndexedVadEvent {
            event: super::VadEvent::SpeechEnd {
                voiced_ms, emit, ..
            },
            ..
        }] => {
            assert_eq!(*voiced_ms, 120, "voiced time carries across pushes");
            assert!(emit);
        }
        other => panic!("expected a single speech end, got {other:?}"),
    }

    session.reset(&config).await.expect("VadReset");
    session.close(&config).await.expect("VadClose");
}

#[test]
fn an_out_of_range_volume_is_clamped_at_the_boundary() {
    // `percent` is interpolated into an `osascript` command downstream, and
    // this type is decoded from a wire payload. The module clamps too, so this
    // is belt-and-braces — but the braces are three lines and the failure mode
    // is a shell command carrying a number nobody checked.
    let decoded: VoiceIntent =
        serde_json::from_str(r#"{"intent":"set_volume","percent":255}"#).expect("decodes");
    assert_eq!(decoded, VoiceIntent::SetVolume { percent: 255 });
    assert_eq!(
        clamped(decoded.clone()),
        VoiceIntent::SetVolume { percent: 100 },
        "the clamp is what `route` applies before any caller sees the intent"
    );

    // In-range values are untouched, including the boundary itself.
    for percent in [0u8, 1, 50, 100] {
        let intent = VoiceIntent::SetVolume { percent };
        assert_eq!(clamped(intent.clone()), intent);
    }
}

#[tokio::test]
async fn every_entry_point_degrades_rather_than_hanging_when_the_module_is_gone() {
    // The fallback directions are asymmetric on purpose, and the asymmetry is
    // the point: whichever way a caller falls, it must first get a plain error
    // back rather than a panic or a stall. `ops.rs` falls OPEN on this error
    // (failing closed would silently delete real speech) and `always_on`'s
    // wake-word gate falls CLOSED (failing open would hand an unaddressed
    // utterance to the agent). Both depend on seeing `Unavailable`.
    let mut config = offline_config();
    config.modules.enabled = false;

    assert!(matches!(
        super::is_hallucinated(
            &config,
            "thank you for watching",
            HallucinationMode::Conversation
        )
        .await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::extract_command(&config, "hey tiny pause", "Hey Tiny").await,
        Err(VoiceCallError::Unavailable(_))
    ));

    // The session surface has to reach the same verdict, because always_on now
    // retries `open` on a cooldown instead of tearing the loop down: a variant
    // other than `Unavailable` here would change that path's behaviour.
    assert!(matches!(
        super::VadSession::open(
            &config,
            super::vad_config_from_server_config(
                &crate::openhuman::config::VoiceServerConfig::default()
            )
        )
        .await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::prepare_frames(&config, &[0.1, 0.2], 32_000, 2).await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::frame_energies(&config, &[0.1, 0.2], 1).await,
        Err(VoiceCallError::Unavailable(_))
    ));
    assert!(matches!(
        super::encode_wav_pcm16(&config, &[1i16, 2], 16_000, 1).await,
        Err(VoiceCallError::Unavailable(_))
    ));
}

#[test]
fn every_member_this_client_calls_is_one_the_contract_declares() {
    // The fifteen call sites in this module are written as `tinyvoice_bus`
    // constants, so a rename upstream is a compile error here rather than a
    // `MemberNotFound` at runtime. This pins the other direction: a member the
    // contract declares and this client never calls is either a gap in the
    // client or a member that should not be in the contract, and either way it
    // should be noticed here rather than discovered later.
    use tinyvoice_bus::names::methods;
    let called = [
        methods::ROUTE,
        methods::EXTRACT_COMMAND,
        methods::WAKE_WORD_PRESENT,
        methods::IS_HALLUCINATED,
        methods::VAD_OPEN,
        methods::VAD_PUSH,
        methods::VAD_IS_SPEAKING,
        methods::VAD_RESET,
        methods::VAD_CLOSE,
        methods::PREPARE_FRAMES,
        methods::FRAME_ENERGIES,
        methods::ENCODE_WAV,
        methods::ENCODE_WAV_PCM16,
        methods::PREPARE_CAPTURE,
    ];
    // `Segment` is the one deliberate omission: it segments a complete energy
    // buffer in one call, and the always-on capture loop needs the stateful
    // `Vad*` session instead, because a segmenter is a state machine across
    // frames that arrive one at a time.
    for member in tinyvoice_bus::names::METHODS {
        if *member == methods::SEGMENT {
            continue;
        }
        assert!(
            called.contains(member),
            "the contract declares `{member}`, which this client never calls"
        );
    }
}
