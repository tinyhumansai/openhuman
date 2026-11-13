use super::*;

/// The voice-default block `reply_synthesize` and `tts_dispatch` each carried
/// inline before they shared `resolve_tts_voice`, kept as the oracle.
fn inline_voice_default(requested: Option<&str>, provider_name: &str) -> String {
    requested
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if provider_name == "piper" {
                crate::voice::DEFAULT_PIPER_VOICE.to_string()
            } else {
                String::new()
            }
        })
}

#[test]
fn resolve_tts_voice_pins_the_defaults() {
    let piper = crate::voice::DEFAULT_PIPER_VOICE;
    let cases: [(Option<&str>, &str, &str); 10] = [
        (None, "piper", piper),
        (Some(""), "piper", piper),
        (Some("   "), "piper", piper),
        (Some(" en_GB-alan-low "), "piper", "en_GB-alan-low"),
        (None, "cloud", ""),
        (Some("  "), "cloud", ""),
        (Some("alloy"), "cloud", "alloy"),
        (None, "elevenlabs:eleven_turbo", ""),
        (Some("\tvoice-1\n"), "elevenlabs:eleven_turbo", "voice-1"),
        // Only the exact slug is Piper; a model-qualified or padded name is not.
        (None, " piper", ""),
    ];
    for (requested, provider, expected) in cases {
        assert_eq!(
            resolve_tts_voice(requested, provider),
            expected,
            "requested={requested:?} provider={provider:?}"
        );
        assert_eq!(
            resolve_tts_voice(requested, provider),
            inline_voice_default(requested, provider),
            "helper must match the former inline block for requested={requested:?} provider={provider:?}"
        );
    }
}
