use super::*;
use hotkey::{
    ActivationMode::{Push, Tap},
    HotkeyEvent::{Pressed, Released},
};
use tinycomputer_bus::accessibility::GlobeEvent::{Down, Up};

#[cfg(not(target_os = "macos"))]
use tinyvoice_bus::hotkey::{HotkeyBatch, SequencedHotkeyEvent};

#[cfg(not(target_os = "macos"))]
fn batch(
    generation: u64,
    batch: u64,
    events: &[(u64, hotkey::HotkeyEvent)],
    reset: bool,
    active: bool,
) -> HotkeyBatch {
    HotkeyBatch {
        generation,
        batch,
        events: events
            .iter()
            .map(|(sequence, event)| SequencedHotkeyEvent {
                generation,
                sequence: *sequence,
                event: *event,
            })
            .collect(),
        reset,
        active,
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn native_hotkey_batches_ack_only_after_ordered_event_state_is_valid() {
    let mut delivery = HotkeyDelivery::new(4);
    assert_eq!(
        delivery
            .accept(&batch(4, 1, &[(1, Pressed)], false, true))
            .unwrap(),
        [Pressed]
    );
    assert_eq!(delivery.acknowledged_batch, Some(1));

    let invalid = batch(4, 2, &[(2, Released)], false, true);
    assert!(delivery.accept(&invalid).is_err());
    assert_eq!(delivery.acknowledged_batch, Some(1));
    assert!(delivery.active);

    assert_eq!(
        delivery
            .accept(&batch(4, 2, &[(2, Released)], false, false))
            .unwrap(),
        [Released]
    );
    assert_eq!(delivery.acknowledged_batch, Some(2));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn native_hotkey_reset_releases_active_state_and_accepts_fresh_sequence() {
    let mut delivery = HotkeyDelivery::new(9);
    delivery
        .accept(&batch(9, 1, &[(4, Pressed)], false, true))
        .unwrap();
    assert_eq!(
        delivery.accept(&batch(9, 2, &[], true, false)).unwrap(),
        [Released]
    );
    assert_eq!(delivery.acknowledged_batch, Some(2));
    assert_eq!(
        delivery
            .accept(&batch(9, 3, &[(8, Pressed)], false, true))
            .unwrap(),
        [Pressed]
    );
    assert_eq!(delivery.acknowledged_batch, Some(3));
    assert!(delivery.active);
}

#[test]
fn overflow_stops_activation_and_waits_for_physical_release() {
    for mode in [Push, Tap] {
        let mut state = GlobeActivation::default();
        assert!(matches!(state.event(Down, mode), Some(Pressed)));
        assert!(matches!(state.reset(), Some(Released)));
        assert!(state.event(Down, mode).is_none());
        assert!(state.event(Up, mode).is_none());
        assert!(matches!(state.event(Down, mode), Some(Pressed)));
    }
}

#[test]
fn tap_ignores_key_release_and_toggles_on_the_next_press() {
    let mut state = GlobeActivation::default();
    assert!(matches!(state.event(Down, Tap), Some(Pressed)));
    assert!(state.event(Up, Tap).is_none());
    assert!(matches!(state.event(Down, Tap), Some(Released)));
}

#[test]
fn push_suppresses_duplicate_physical_edges() {
    let mut state = GlobeActivation::default();
    assert!(matches!(state.event(Down, Push), Some(Pressed)));
    assert!(state.event(Down, Push).is_none());
    assert!(matches!(state.event(Up, Push), Some(Released)));
    assert!(state.event(Up, Push).is_none());
}

#[cfg(feature = "modules")]
#[derive(Clone, Default)]
struct ListenerFixture {
    entered: std::sync::Arc<tokio::sync::Notify>,
    allow_start: std::sync::Arc<tokio::sync::Notify>,
    stopped: std::sync::Arc<tokio::sync::Notify>,
    closes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    lose_start_reply: std::sync::Arc<std::sync::atomic::AtomicBool>,
    fail_close: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "modules")]
fn status(running: bool) -> crate::modules::accessibility::GlobeHotkeyStatus {
    crate::modules::accessibility::GlobeHotkeyStatus {
        supported: true,
        running,
        input_monitoring_permission: crate::modules::accessibility::PermissionState::Granted,
        last_error: None,
        events_pending: 0,
    }
}

#[cfg(feature = "modules")]
#[tinybus::interface(name = "test.Globe")]
impl ListenerFixture {
    async fn globe_start(&self) -> tinybus::Result<tinycomputer_bus::DesktopResponse> {
        self.entered.notify_one();
        if self
            .lose_start_reply
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(tinybus::Error::MethodFailed {
                name: tinybus::Error::NOT_ATTESTED.into(),
                message: "fixture lost response".into(),
            });
        }
        self.allow_start.notified().await;
        Ok(tinycomputer_bus::DesktopResponse::ok(
            "globe-start",
            serde_json::to_value(crate::modules::accessibility::GlobeStarted {
                handle: crate::modules::accessibility::GlobeHandle("lease".into()),
                status: status(true),
            })
            .unwrap(),
        ))
    }
    async fn globe_stop(
        &self,
        handle: crate::modules::accessibility::GlobeHandle,
    ) -> tinybus::Result<tinycomputer_bus::DesktopResponse> {
        assert_eq!(handle.0, "lease");
        if self
            .fail_close
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(tinybus::Error::failed("fixture cleanup pending"));
        }
        self.closes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.stopped.notify_one();
        Ok(tinycomputer_bus::DesktopResponse::ok(
            "globe-stop",
            serde_json::to_value(status(false)).unwrap(),
        ))
    }
    async fn globe_read(
        &self,
        request: crate::modules::accessibility::GlobeRead,
    ) -> tinybus::Result<tinycomputer_bus::DesktopResponse> {
        Ok(tinycomputer_bus::DesktopResponse::ok(
            "globe-read",
            serde_json::to_value(crate::modules::accessibility::GlobeBatch {
                handle: request.handle,
                batch: 1,
                status: status(true),
                events: vec![],
                overflow: false,
            })
            .unwrap(),
        ))
    }
}

#[cfg(feature = "modules")]
async fn listener_fixture() -> (
    crate::modules::accessibility::AccessibilityClient,
    ListenerFixture,
    tinybus::Connection,
) {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let fixture = ListenerFixture::default();
    service
        .serve_at(ObjectPath::new("/test/Globe").unwrap(), fixture.clone())
        .await
        .unwrap();
    service.request_name("test.Globe").await.unwrap();
    let proxy = caller
        .proxy("test.Globe", "/test/Globe", "test.Globe")
        .unwrap();
    (
        crate::modules::accessibility::AccessibilityClient::fixture(proxy),
        fixture,
        service,
    )
}

#[cfg(feature = "modules")]
async fn cancelling_startup_still_closes_the_module_owned_listener_fixture() {
    let (client, fixture, _service) = listener_fixture().await;
    let startup = tokio::spawn(async move {
        start_globe_with_client(
            &crate::core::runtime::DispatchCtx::default(),
            client,
            Push,
            &CancellationToken::new(),
        )
        .await
    });
    fixture.entered.notified().await;
    startup.abort();
    assert!(matches!(startup.await, Err(error) if error.is_cancelled()));
    fixture.allow_start.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.stopped.notified(),
    )
    .await
    .unwrap();
    assert_eq!(fixture.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[cfg(feature = "modules")]
async fn stopping_listener_joins_its_bus_cleanup_fixture() {
    let (client, fixture, _service) = listener_fixture().await;
    fixture.allow_start.notify_one();
    let (handle, _rx) = start_globe_with_client(
        &crate::core::runtime::DispatchCtx::default(),
        client,
        Push,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    handle.stop().await;
    assert_eq!(fixture.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[cfg(feature = "modules")]
async fn lost_start_reply_recovers_the_lease_before_releasing_admission_fixture() {
    let (client, fixture, _service) = listener_fixture().await;
    fixture
        .lose_start_reply
        .store(true, std::sync::atomic::Ordering::SeqCst);
    fixture.allow_start.notify_one();
    assert!(start_globe_with_client(
        &crate::core::runtime::DispatchCtx::default(),
        client,
        Push,
        &CancellationToken::new()
    )
    .await
    .is_err());
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.stopped.notified(),
    )
    .await
    .unwrap();
    assert_eq!(fixture.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[cfg(feature = "modules")]
async fn failed_close_retains_ownership_until_the_module_confirms_cleanup_fixture() {
    let (client, fixture, _service) = listener_fixture().await;
    fixture
        .fail_close
        .store(true, std::sync::atomic::Ordering::SeqCst);
    fixture.allow_start.notify_one();
    let (handle, _rx) = start_globe_with_client(
        &crate::core::runtime::DispatchCtx::default(),
        client,
        Push,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    handle.stop().await;
    assert_eq!(fixture.closes.load(std::sync::atomic::Ordering::SeqCst), 1);
}

// Each fixture owns the process-wide Globe admission lock and its cleanup tasks.
// Separate processes prevent paused-clock fixtures from timing out on another
// fixture's in-flight cleanup, and runtime teardown cannot strand a shared lock.
#[cfg(feature = "modules")]
fn run_lifecycle_fixture(name: &str, fixture: impl std::future::Future<Output = ()>) {
    const CHILD_ENV: &str = "OPENHUMAN_HOTKEY_LIFECYCLE_FIXTURE";
    if std::env::var(CHILD_ENV).as_deref() == Ok(name) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(
                name == "failed_close_retains_ownership_until_the_module_confirms_cleanup",
            )
            .build()
            .unwrap()
            .block_on(fixture);
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "--nocapture"])
        .env(CHILD_ENV, name)
        .arg(format!("voice::server::hotkey_listener::tests::{name}"))
        .output()
        .unwrap();
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "lifecycle fixture failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[cfg(feature = "modules")]
#[test]
fn cancelling_startup_still_closes_the_module_owned_listener() {
    run_lifecycle_fixture(
        "cancelling_startup_still_closes_the_module_owned_listener",
        cancelling_startup_still_closes_the_module_owned_listener_fixture(),
    );
}

#[cfg(feature = "modules")]
#[test]
fn stopping_listener_joins_its_bus_cleanup() {
    run_lifecycle_fixture(
        "stopping_listener_joins_its_bus_cleanup",
        stopping_listener_joins_its_bus_cleanup_fixture(),
    );
}

#[cfg(feature = "modules")]
#[test]
fn lost_start_reply_recovers_the_lease_before_releasing_admission() {
    run_lifecycle_fixture(
        "lost_start_reply_recovers_the_lease_before_releasing_admission",
        lost_start_reply_recovers_the_lease_before_releasing_admission_fixture(),
    );
}

#[cfg(feature = "modules")]
#[test]
fn failed_close_retains_ownership_until_the_module_confirms_cleanup() {
    run_lifecycle_fixture(
        "failed_close_retains_ownership_until_the_module_confirms_cleanup",
        failed_close_retains_ownership_until_the_module_confirms_cleanup_fixture(),
    );
}
