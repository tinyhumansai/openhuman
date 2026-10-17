//! Host recording policy over module-owned native capture and audio preparation.
use super::capture_lease::{CaptureLease, Kind};
use crate::config::Config;
/// Sample rate of the TinyVoice output contract (mono 16 kHz PCM).
pub(crate) const TARGET_SAMPLE_RATE: u32 = 16_000;
/// Prepared recording and module-computed metrics used by the STT gates.
#[derive(Debug, Clone)]
pub struct RecordingResult {
    /// Prepared WAV audio.
    pub wav_bytes: Vec<u8>,
    /// Duration of the gated output.
    pub duration_secs: f32,
    /// Mono sample count after gating.
    pub sample_count: usize,
    /// Peak RMS measured before gating.
    pub peak_rms: f32,
}
/// Opaque recording lease. Dropping it cancels capture and releases output.
pub struct RecordingHandle {
    inner: Option<CaptureLease>,
    #[cfg(test)]
    finalized: Option<Result<RecordingResult, String>>,
}
impl RecordingHandle {
    /// Stop capture, read prepared audio in bounded slices, and release it.
    pub async fn stop(mut self, _config: &Config) -> Result<RecordingResult, String> {
        #[cfg(test)]
        if let Some(result) = self.finalized.take() {
            return result;
        }
        let lease = self.inner.take().ok_or("recording already stopped")?;
        let result = lease.finish().await;
        if result.is_err() {
            if let Err(error) = lease.close().await {
                log::warn!("[voice_capture] recording cleanup pending: {error}");
            }
        }
        result
    }
    #[cfg(test)]
    pub(crate) fn from_test_result(result: Result<RecordingResult, String>) -> Self {
        Self {
            inner: None,
            finalized: Some(result),
        }
    }
}
/// Obtain permission through TinyComputer before starting a TinyVoice recording.
pub async fn start_recording(config: &Config) -> Result<RecordingHandle, String> {
    crate::modules::accessibility::AccessibilityClient::new(config)
        .microphone_permission(true)
        .await
        .map_err(|e| e.to_string())?;
    let lease = CaptureLease::reserve(config, Kind::Recording).await?;
    lease.start_recording().await?;
    Ok(RecordingHandle {
        inner: Some(lease),
        #[cfg(test)]
        finalized: None,
    })
}
/// Enumerate native input devices through the loaded module.
pub async fn list_input_devices(config: &Config) -> Result<Vec<String>, String> {
    use tinyvoice_bus::{capture::CaptureResult, names::methods};
    let result: CaptureResult<Vec<String>> = crate::modules::client(config.clone())
        .call_confidential("tinyvoice", methods::LIST_INPUT_DEVICES, ())
        .await
        .map_err(|e| e.to_string())?;
    result.map_err(|_| "input device enumeration failed".to_string())
}
