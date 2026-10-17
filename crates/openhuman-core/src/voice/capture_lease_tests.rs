use super::*;

#[tokio::test]
async fn missing_module_never_starts_a_local_microphone() {
    let mut config = Config::default();
    config.modules.enabled = false;
    let error = CaptureLease::reserve(&config, Kind::Recording)
        .await
        .err()
        .unwrap();
    assert!(error.contains("MODULE_CALL_REPORTED"));
}

#[tokio::test]
async fn unavailable_cleanup_retains_the_lease_for_retry() {
    let port = Arc::new(Fixture::default());
    port.fail_cleanup.store(true, Ordering::SeqCst);
    let lease = fixture_lease(port.clone(), Kind::Recording);
    assert!(lease.close().await.is_err());
    assert!(!lease.state.closed.load(Ordering::SeqCst));
    port.fail_cleanup.store(false, Ordering::SeqCst);
    lease.close().await.unwrap();
    assert!(lease.state.closed.load(Ordering::SeqCst));
    assert_eq!(
        *port.calls.lock().unwrap(),
        vec![
            methods::RECORDING_CANCEL,
            methods::RECORDING_CANCEL,
            methods::RELEASE_AUDIO_OUTPUT
        ]
    );
}

#[tokio::test]
async fn finishing_reads_bounded_output_and_uses_module_metrics() {
    let port = Arc::new(Fixture::default());
    let lease = fixture_lease(port.clone(), Kind::Recording);
    let result = lease.finish().await.unwrap();
    assert_eq!(result.wav_bytes, vec![7; READ_BYTES + 2]);
    assert_eq!(result.sample_count, (READ_BYTES + 2 - 44) / 2);
    assert_eq!(result.peak_rms, 0.25);
    assert_eq!(
        port.reads.lock().unwrap().as_slice(),
        &[(0, READ_BYTES), (READ_BYTES, 2)]
    );
    assert!(lease.state.closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancelled_finish_releases_the_known_recording_and_output() {
    let port = Arc::new(Fixture::default());
    port.block_finish.store(true, Ordering::SeqCst);
    let lease = fixture_lease(port.clone(), Kind::Recording);
    let state = lease.state.clone();
    let task = tokio::spawn(async move { lease.finish().await });
    port.entered.notified().await;
    task.abort();
    let _ = task.await;
    state.close().await.unwrap();
    assert!(state.closed.load(Ordering::SeqCst));
    let calls = port.calls.lock().unwrap();
    assert!(calls.contains(&methods::RECORDING_CANCEL));
    assert!(calls.contains(&methods::RELEASE_AUDIO_OUTPUT));
}

#[tokio::test]
async fn closed_stream_cannot_poll_or_restart_after_sign_out() {
    let port = Arc::new(Fixture::default());
    let lease = fixture_lease(port.clone(), Kind::Stream);
    lease.close().await.unwrap();
    assert!(lease.poll().await.is_err());
    assert!(lease.start_stream().await.is_err());
    assert_eq!(*port.calls.lock().unwrap(), vec![methods::CAPTURE_STOP]);
}

#[derive(Default)]
struct Fixture {
    calls: std::sync::Mutex<Vec<&'static str>>,
    reads: std::sync::Mutex<Vec<(usize, usize)>>,
    fail_cleanup: AtomicBool,
    block_finish: AtomicBool,
    block_reserve: AtomicBool,
    release_reserve: tokio::sync::Notify,
    entered: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl Port for Fixture {
    async fn call(
        &self,
        member: &'static str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.calls.lock().unwrap().push(member);
        if self.fail_cleanup.load(Ordering::SeqCst) && member == methods::RECORDING_CANCEL {
            return Err("MODULE_CALL_REPORTED: transport failed".into());
        }
        match member {
            methods::RESERVE_CAPTURE => {
                if self.block_reserve.load(Ordering::SeqCst) {
                    self.entered.notify_one();
                    self.release_reserve.notified().await;
                }
                Ok(serde_json::to_value(CaptureResult::Ok(CaptureHandle("lease".into()))).unwrap())
            }

            methods::RECORDING_FINISH => {
                if self.block_finish.load(Ordering::SeqCst) {
                    self.entered.notify_one();
                    std::future::pending::<()>().await;
                }
                Ok(serde_json::to_value(CaptureResult::Ok(AudioOutput {
                    handle: CaptureHandle("lease".into()),
                    length: READ_BYTES + 2,
                }))
                .unwrap())
            }
            methods::RECORDING_SUMMARY => {
                Ok(serde_json::to_value(CaptureResult::Ok(RecordingSummary {
                    sample_count: (READ_BYTES + 2 - 44) / 2,
                    peak_rms: 0.25,
                }))
                .unwrap())
            }
            methods::READ_AUDIO_OUTPUT => {
                let request: ReadAudioRequest = serde_json::from_value(args[0].clone()).unwrap();
                self.reads
                    .lock()
                    .unwrap()
                    .push((request.offset, request.length));
                Ok(serde_json::to_value(CaptureResult::Ok(
                    base64::engine::general_purpose::STANDARD.encode(vec![7; request.length]),
                ))
                .unwrap())
            }
            methods::RECORDING_CANCEL => Ok(serde_json::to_value(CaptureResult::<()>::Err(
                CaptureError::UnknownHandle,
            ))
            .unwrap()),
            _ => Ok(serde_json::to_value(CaptureResult::Ok(())).unwrap()),
        }
    }
}
fn fixture_lease(port: Arc<Fixture>, kind: Kind) -> CaptureLease {
    CaptureLease {
        state: Arc::new(State {
            id: 0,
            port,
            handle: CaptureHandle("lease".into()),
            kind,
            closed: AtomicBool::new(false),
            serial: tokio::sync::Mutex::new(()),
        }),
    }
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn capture_never_retries_through_an_unattested_bus() {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    #[derive(Clone)]
    struct Unattested(Arc<AtomicU64>);
    #[tinybus::interface(name = "test.Capture")]
    impl Unattested {
        async fn recording_start(
            &self,
            _: RecordingStartRequest,
        ) -> tinybus::Result<CaptureResult<CaptureHandle>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Ok(CaptureHandle("lease".into())))
        }
    }
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let calls = Arc::new(AtomicU64::new(0));
    service
        .serve_at(
            ObjectPath::new("/test/Capture").unwrap(),
            Unattested(calls.clone()),
        )
        .await
        .unwrap();
    service.request_name("test.Capture").await.unwrap();
    let proxy = caller
        .proxy("test.Capture", "/test/Capture", "test.Capture")
        .unwrap();
    let lease = CaptureLease {
        state: Arc::new(State {
            id: 0,
            port: Arc::new(BusPort(crate::modules::ops::fixture_client(proxy))),
            handle: CaptureHandle("lease".into()),
            kind: Kind::Recording,
            closed: AtomicBool::new(false),
            serial: tokio::sync::Mutex::new(()),
        }),
    };
    let error = lease.start_recording().await.unwrap_err();
    assert!(error.contains("MODULE_CALL_REPORTED"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    lease.state.closed.store(true, Ordering::SeqCst);
}

#[tokio::test]
async fn a_reservation_finishing_after_sign_out_is_closed_before_start() {
    let port = Arc::new(Fixture::default());
    port.block_reserve.store(true, Ordering::SeqCst);
    let task_port = port.clone();
    let task =
        tokio::spawn(async move { CaptureLease::reserve_with_port(task_port, Kind::Stream).await });
    port.entered.notified().await;
    Kind::Stream.generation().fetch_add(1, Ordering::SeqCst);
    port.release_reserve.notify_one();
    assert!(task.await.unwrap().is_err());
    assert_eq!(
        *port.calls.lock().unwrap(),
        vec![methods::RESERVE_CAPTURE, methods::CAPTURE_STOP]
    );
}
