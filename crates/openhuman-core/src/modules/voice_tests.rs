use super::*;

#[tokio::test]
async fn transcript_screening_reports_unavailable_when_modules_are_disabled() {
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;

    assert!(matches!(
        is_hallucinated(&config, "recognized words", HallucinationMode::Conversation).await,
        Err(VoiceCallError::Unavailable(_))
    ));
}

#[test]
fn tinyvoice_module_remains_registered_for_transcript_screening() {
    let record = crate::modules::registry::find("tinyvoice").expect("tinyvoice is registered");
    assert_eq!(record.bus_name, tinyvoice_bus::names::BUS_NAME);
    assert_eq!(record.object_path, tinyvoice_bus::names::OBJECT_PATH);
}

#[tokio::test]
async fn wav_encoding_reports_unavailable_when_modules_are_disabled() {
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;

    assert!(matches!(
        encode_wav_pcm16(&config, &[0; 4], 16_000, 1, None).await,
        Err(VoiceCallError::Unavailable(_))
    ));
}

#[test]
fn pcm16_samples_cross_the_bus_as_little_endian_bytes() {
    use base64::Engine;
    let decode = |b64: String| {
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap()
    };

    assert_eq!(
        decode(pcm16_le_base64(&[1, -1, 32767, i16::MIN])),
        [0x01, 0x00, 0xff, 0xff, 0xff, 0x7f, 0x00, 0x80]
    );
    assert_eq!(pcm16_le_base64(&[]), "");
}

#[tokio::test]
async fn a_bounded_encode_reports_still_loading_while_the_module_downloads() {
    // Hold the process-wide resolution slot in flight, as a first-run download
    // does, so the bounded wait passes before the module is ready.
    let table = tinybus::module::resolution::global();
    table.forget(MODULE_ID);
    let _sender = table.mark_in_flight(MODULE_ID);

    let result = encode_wav_pcm16(
        &crate::config::Config::default(),
        &[0; 4],
        16_000,
        1,
        Some(std::time::Duration::ZERO),
    )
    .await;
    table.forget(MODULE_ID);

    assert_eq!(
        result,
        Err(VoiceCallError::Unavailable(
            STILL_LOADING_MESSAGE.to_string()
        ))
    );
}
