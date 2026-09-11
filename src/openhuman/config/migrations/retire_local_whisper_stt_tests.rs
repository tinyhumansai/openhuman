use super::*;
// `voice::factory` is behind the `voice` gate, so the two end-to-end
// assertions below only compile in a voice-enabled build. The migration
// itself is ungated and its own tests run in every configuration.
#[cfg(feature = "voice")]
use crate::openhuman::config::schema::SttEngine;
#[cfg(feature = "voice")]
use crate::openhuman::voice::factory::effective_stt_provider;

fn config_with(stt_provider: Option<&str>, legacy: &str) -> Config {
    let mut config = Config::default();
    config.stt_provider = stt_provider.map(str::to_string);
    config.local_ai.stt_provider = legacy.to_string();
    config
}

#[test]
fn rewrites_both_fields_when_they_name_the_removed_engine() {
    let mut config = config_with(Some("whisper"), "whisper");
    let stats = run(&mut config).expect("migration is infallible");

    assert!(stats.stt_provider_migrated);
    assert!(stats.legacy_stt_provider_migrated);
    assert_eq!(config.stt_provider.as_deref(), Some("cloud"));
    assert_eq!(config.local_ai.stt_provider, "cloud");
}

#[test]
fn matches_the_local_alias_and_ignores_case_and_padding() {
    let mut config = config_with(Some("  Whisper  "), "LOCAL");
    run(&mut config).expect("migration is infallible");

    assert_eq!(config.stt_provider.as_deref(), Some("cloud"));
    assert_eq!(config.local_ai.stt_provider, "cloud");
}

/// The migration must only ever move a broken config to a working one — a
/// third-party slug is a deliberate choice and rewriting it would silently move
/// the user's transcription (and billing) onto a different provider.
#[test]
fn leaves_third_party_and_cloud_selections_untouched() {
    let mut config = config_with(Some("deepgram:nova-2"), "cloud");
    let stats = run(&mut config).expect("migration is infallible");

    assert_eq!(stats, MigrationStats::default());
    assert_eq!(config.stt_provider.as_deref(), Some("deepgram:nova-2"));
    assert_eq!(config.local_ai.stt_provider, "cloud");
}

#[test]
fn leaves_an_unset_provider_unset() {
    let mut config = config_with(None, "cloud");
    run(&mut config).expect("migration is infallible");
    assert!(config.stt_provider.is_none());
}

#[test]
fn is_idempotent() {
    let mut config = config_with(Some("whisper"), "whisper");
    run(&mut config).expect("first run");
    let second = run(&mut config).expect("second run");
    assert_eq!(second, MigrationStats::default());
}

/// A user who already picked a hosted engine keeps it: the migration clears the
/// dead routing string without overwriting the engine selection.
#[cfg(feature = "voice")]
#[test]
fn preserves_an_explicitly_chosen_engine() {
    let mut config = config_with(Some("whisper"), "whisper");
    config.voice_server.stt_engine = SttEngine::Elevenlabs;
    run(&mut config).expect("migration is infallible");

    assert_eq!(config.voice_server.stt_engine, SttEngine::Elevenlabs);
    assert_eq!(effective_stt_provider(&config), "elevenlabs");
}

/// End-to-end: the point of the migration is that the factory can resolve the
/// result. Before it, `"whisper"` reached the slug lookup and errored.
#[cfg(feature = "voice")]
#[test]
fn migrated_config_resolves_to_the_hosted_backend() {
    let mut config = config_with(Some("whisper"), "whisper");
    run(&mut config).expect("migration is infallible");
    assert_eq!(effective_stt_provider(&config), "cloud");
}
