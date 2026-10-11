use super::*;
use std::sync::atomic::Ordering;

#[test]
fn stop_clears_enabled_gate() {
    // stop() flips the runtime gate off so the processor drops all audio.
    ENABLED.store(true, Ordering::SeqCst);
    stop();
    assert!(
        !ENABLED.load(Ordering::SeqCst),
        "stop() must clear the runtime gate"
    );
}

// The VAD state machine and the wake-word gate moved to `tinyvoice`, which
// carries their unit tests. What stays testable here is the piece that is
// genuinely OpenHuman's: turning persisted config into the module's tuning.
#[test]
fn vad_config_maps_persisted_seconds_to_milliseconds() {
    let mut c = crate::config::VoiceServerConfig::default();
    c.vad_max_utterance_secs = 2.5;
    c.vad_hangover_ms = 750;

    let v = tinyvoice::vad_config_from_server_config(&c);

    assert_eq!(v.max_utterance_ms, 2500, "seconds become milliseconds");
    assert_eq!(v.hangover_ms, 750, "milliseconds pass through");
    assert_eq!(v.onset_threshold, c.vad_onset_threshold);
}

#[test]
fn a_nonpositive_utterance_ceiling_cannot_collapse_to_zero() {
    // A zero ceiling would close every utterance on its first frame, which
    // reads to a user as the microphone hearing nothing at all.
    let mut c = crate::config::VoiceServerConfig::default();
    c.vad_max_utterance_secs = 0.0;
    assert_eq!(
        tinyvoice::vad_config_from_server_config(&c).max_utterance_ms,
        1
    );

    c.vad_max_utterance_secs = -5.0;
    assert_eq!(
        tinyvoice::vad_config_from_server_config(&c).max_utterance_ms,
        1
    );
}

#[tokio::test]
async fn an_utterance_started_after_stop_never_polls_its_transcription() {
    let (stop, observed) = tokio::sync::watch::channel(0_u64);
    let delivered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let task_delivered = delivered.clone();
    stop.send_replace(1);
    let mut tasks = tokio::task::JoinSet::new();
    processor::spawn_transcription(&mut tasks, &observed, async move {
        task_delivered.store(true, Ordering::SeqCst);
    });
    tasks.join_next().await.unwrap().unwrap();
    assert!(!delivered.load(Ordering::SeqCst));
}
