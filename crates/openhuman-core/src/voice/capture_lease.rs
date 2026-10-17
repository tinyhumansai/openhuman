//! Host ownership and cancellation of opaque TinyVoice capture leases.
use super::audio_capture::{RecordingResult, TARGET_SAMPLE_RATE};
use crate::config::Config;
use base64::Engine as _;
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, LazyLock, Mutex,
};
use tinybus::module::ModuleClient;
use tinyvoice_bus::{
    capture::{
        AudioOutput, CaptureBatch, CaptureError, CaptureHandle, CapturePollRequest, CaptureResult,
        CaptureStream, MicrophonePermission, ReadAudioRequest, RecordingFinishRequest,
        RecordingStartRequest, RecordingSummary, RecordingSummaryRequest,
    },
    names::methods,
};

const MODULE: &str = "tinyvoice";
const READ_BYTES: usize = 256 * 1024;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
static RECORDING_GENERATION: AtomicU64 = AtomicU64::new(0);
static STREAM_GENERATION: AtomicU64 = AtomicU64::new(0);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static LEASES: LazyLock<Mutex<HashMap<u64, Arc<State>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static HOOK: AtomicBool = AtomicBool::new(false);
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Recording,
    Stream,
}
impl Kind {
    fn generation(self) -> &'static AtomicU64 {
        match self {
            Self::Recording => &RECORDING_GENERATION,
            Self::Stream => &STREAM_GENERATION,
        }
    }
}
#[async_trait::async_trait]
trait Port: Send + Sync {
    async fn call(
        &self,
        member: &'static str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}
struct BusPort(ModuleClient);
#[async_trait::async_trait]
impl Port for BusPort {
    async fn call(
        &self,
        member: &'static str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.0
            .call_confidential(MODULE, member, args)
            .await
            .map_err(|e| e.to_string())
    }
}
struct State {
    id: u64,
    port: Arc<dyn Port>,
    handle: CaptureHandle,
    kind: Kind,
    closed: AtomicBool,
    serial: tokio::sync::Mutex<()>,
}
impl State {
    async fn call<T: DeserializeOwned>(
        &self,
        member: &'static str,
        args: impl Serialize,
    ) -> Result<CaptureResult<T>, String> {
        let args = serde_json::to_value(args).map_err(|_| malformed())?;
        let value = self.port.call(member, args).await?;
        serde_json::from_value(value).map_err(|_| malformed())
    }
    fn check_open(&self) -> Result<(), String> {
        if self.closed.load(Ordering::SeqCst) {
            Err("capture lease closed".into())
        } else {
            Ok(())
        }
    }
    async fn close(&self) -> Result<(), String> {
        let _serial = self.serial.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Ok(());
        }
        match self.kind {
            Kind::Recording => {
                cleanup_result(
                    self.call::<()>(methods::RECORDING_CANCEL, (&self.handle,))
                        .await?,
                )?;
                cleanup_result(
                    self.call::<()>(methods::RELEASE_AUDIO_OUTPUT, (&self.handle,))
                        .await?,
                )?;
            }
            Kind::Stream => cleanup_result(
                self.call::<()>(methods::CAPTURE_STOP, (&self.handle,))
                    .await?,
            )?,
        }
        self.mark_closed();
        Ok(())
    }
    fn mark_closed(&self) {
        self.closed.store(true, Ordering::SeqCst);
        LEASES
            .lock()
            .expect("capture lease registry poisoned")
            .remove(&self.id);
    }
}
/// Dropping a lease schedules cleanup; failed cleanup stays registered for retry.
pub(super) struct CaptureLease {
    state: Arc<State>,
}
impl CaptureLease {
    pub(super) async fn reserve(config: &Config, kind: Kind) -> Result<Self, String> {
        let port: Arc<dyn Port> = Arc::new(BusPort(crate::modules::client(config.clone())));
        Self::reserve_with_port(port, kind).await
    }
    async fn reserve_with_port(port: Arc<dyn Port>, kind: Kind) -> Result<Self, String> {
        let generation = kind.generation().load(Ordering::SeqCst);
        let reply = port
            .call(
                methods::RESERVE_CAPTURE,
                serde_json::to_value((RecordingStartRequest {
                    permission: MicrophonePermission::Granted,
                    handle: None,
                },))
                .map_err(|_| malformed())?,
            )
            .await?;
        let handle: CaptureResult<CaptureHandle> =
            serde_json::from_value(reply).map_err(|_| malformed())?;
        let state = Arc::new(State {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            port,
            handle: handle.map_err(capture_error)?,
            kind,
            closed: AtomicBool::new(false),
            serial: tokio::sync::Mutex::new(()),
        });
        LEASES
            .lock()
            .expect("capture lease registry poisoned")
            .insert(state.id, state.clone());
        register_shutdown();
        let lease = Self { state };
        if generation != kind.generation().load(Ordering::SeqCst) {
            lease.close().await?;
            return Err("capture cancelled during lifecycle change".into());
        }
        Ok(lease)
    }
    pub(super) async fn start_recording(&self) -> Result<(), String> {
        let _serial = self.state.serial.lock().await;
        self.state.check_open()?;
        let handle: CaptureHandle = self
            .state
            .call(
                methods::RECORDING_START,
                (RecordingStartRequest {
                    permission: MicrophonePermission::Granted,
                    handle: Some(self.state.handle.clone()),
                },),
            )
            .await?
            .map_err(capture_error)?;
        if handle != self.state.handle {
            return Err(malformed());
        }
        Ok(())
    }
    pub(super) async fn start_stream(&self) -> Result<CaptureStream, String> {
        let _serial = self.state.serial.lock().await;
        self.state.check_open()?;
        let stream: CaptureStream = self
            .state
            .call(
                methods::CAPTURE_START,
                (RecordingStartRequest {
                    permission: MicrophonePermission::Granted,
                    handle: Some(self.state.handle.clone()),
                },),
            )
            .await?
            .map_err(capture_error)?;
        if stream.handle != self.state.handle
            || stream.format.source_rate == 0
            || stream.format.channels == 0
        {
            return Err(malformed());
        }
        Ok(stream)
    }
    pub(super) async fn poll(&self) -> Result<CaptureBatch, String> {
        let _serial = self.state.serial.lock().await;
        self.state.check_open()?;
        let batch: CaptureBatch = self
            .state
            .call(
                methods::CAPTURE_POLL,
                (CapturePollRequest {
                    handle: self.state.handle.clone(),
                    max_chunks: 2,
                },),
            )
            .await?
            .map_err(capture_error)?;
        if batch.chunks.len() > 2
            || batch.chunks.iter().any(|chunk| {
                chunk.samples.len() > 32_768
                    || chunk.samples.iter().any(|sample| !sample.is_finite())
            })
        {
            return Err(malformed());
        }
        Ok(batch)
    }
    pub(super) async fn finish(&self) -> Result<RecordingResult, String> {
        let _serial = self.state.serial.lock().await;
        self.state.check_open()?;
        let output: AudioOutput = self
            .state
            .call(
                methods::RECORDING_FINISH,
                (RecordingFinishRequest {
                    handle: self.state.handle.clone(),
                    gate_threshold: 0.002,
                },),
            )
            .await?
            .map_err(capture_error)?;
        if output.handle != self.state.handle || !(44..=MAX_OUTPUT_BYTES).contains(&output.length) {
            return Err(malformed());
        }
        let summary: RecordingSummary = self
            .state
            .call(
                methods::RECORDING_SUMMARY,
                (RecordingSummaryRequest {
                    handle: self.state.handle.clone(),
                },),
            )
            .await?
            .map_err(capture_error)?;
        if !summary.peak_rms.is_finite()
            || summary.peak_rms < 0.0
            || summary.sample_count > MAX_OUTPUT_BYTES / 2
            || summary.sample_count.saturating_mul(2).saturating_add(44) != output.length
        {
            return Err(malformed());
        }
        let mut wav_bytes = Vec::with_capacity(output.length);
        while wav_bytes.len() < output.length {
            let length = READ_BYTES.min(output.length - wav_bytes.len());
            let encoded: String = self
                .state
                .call(
                    methods::READ_AUDIO_OUTPUT,
                    (ReadAudioRequest {
                        handle: self.state.handle.clone(),
                        offset: wav_bytes.len(),
                        length,
                    },),
                )
                .await?
                .map_err(capture_error)?;
            if encoded.len() > READ_BYTES.div_ceil(3) * 4 {
                return Err(malformed());
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| malformed())?;
            if bytes.len() != length {
                return Err(malformed());
            }
            wav_bytes.extend(bytes);
        }
        cleanup_result(
            self.state
                .call::<()>(methods::RELEASE_AUDIO_OUTPUT, (&self.state.handle,))
                .await?,
        )?;
        self.state.mark_closed();
        Ok(RecordingResult {
            wav_bytes,
            duration_secs: summary.sample_count as f32 / TARGET_SAMPLE_RATE as f32,
            sample_count: summary.sample_count,
            peak_rms: summary.peak_rms,
        })
    }
    pub(super) async fn close(&self) -> Result<(), String> {
        self.state.close().await
    }
}
impl Drop for CaptureLease {
    fn drop(&mut self) {
        if self.state.closed.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let state = self.state.clone();
            runtime.spawn(async move {
                if let Err(error) = state.close().await {
                    log::warn!("[voice_capture] lease cleanup pending: {error}");
                }
            });
        }
    }
}
fn malformed() -> String {
    crate::modules::report::malformed_reply(MODULE);
    tinybus::module::ModuleCallError::ModuleFault.to_string()
}
fn capture_error(error: CaptureError) -> String {
    let reason = match error {
        CaptureError::PermissionDenied => "microphone permission denied",
        CaptureError::Cancelled => "capture cancelled",
        CaptureError::Closed => "capture module closed",
        CaptureError::Busy => "microphone already in use",
        CaptureError::UnknownHandle => "capture lease unavailable",
        CaptureError::LimitExceeded => "capture limit exceeded",
        CaptureError::InvalidParameters => "invalid capture parameters",
        CaptureError::Device(_) => "microphone operation failed",
    };
    reason.into()
}
fn cleanup_result(result: CaptureResult<()>) -> Result<(), String> {
    match result {
        Ok(()) | Err(CaptureError::UnknownHandle) => Ok(()),
        Err(error) => Err(capture_error(error)),
    }
}
/// Release every retained capture lease on sign-out, including failed Drop cleanup.
pub(crate) async fn close_all() -> Result<(), String> {
    close_kind(None).await
}
pub(super) async fn close_streams() -> Result<(), String> {
    close_kind(Some(Kind::Stream)).await
}
async fn close_kind(kind: Option<Kind>) -> Result<(), String> {
    for candidate in [Kind::Recording, Kind::Stream] {
        if kind.is_none_or(|kind| kind == candidate) {
            candidate.generation().fetch_add(1, Ordering::SeqCst);
        }
    }
    let states: Vec<_> = LEASES
        .lock()
        .expect("capture lease registry poisoned")
        .values()
        .filter(|state| kind.is_none_or(|kind| kind == state.kind))
        .cloned()
        .collect();
    let mut failure = None;
    for state in states {
        if let Err(error) = state.close().await {
            failure = Some(error);
        }
    }
    failure.map_or(Ok(()), Err)
}
fn register_shutdown() {
    if HOOK.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::core::shutdown::register(|| async {
        shutdown_capture().await;
    });
}
pub(crate) async fn shutdown_capture() {
    super::always_on::shutdown().await;
    HOOK.store(false, Ordering::SeqCst);
    if let Err(error) = close_all().await {
        log::warn!("[voice_capture] shutdown cleanup pending: {error}");
    }
}
#[cfg(test)]
#[path = "capture_lease_tests.rs"]
mod tests;
