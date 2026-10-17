//! Hotkey listener dispatch. TinyVoice owns native listeners; macOS Globe
//! input remains a host-fed composition over TinyComputer facts.

#[cfg(any(target_os = "macos", test))]
use std::time::Duration;

#[cfg(any(target_os = "macos", test))]
use log::warn;
use tokio_util::sync::CancellationToken;

use tinyvoice_bus::hotkey;

use super::LOG_PREFIX;

/// Opaque handle that keeps the hotkey listener alive. Drop to stop.
pub(crate) enum HotkeyListenerKind {
    Module {
        cancel: CancellationToken,
        task: tokio::task::JoinHandle<()>,
    },
    #[cfg(any(target_os = "macos", test))]
    Globe {
        cancel: CancellationToken,
        task: tokio::task::JoinHandle<()>,
    },
}

impl HotkeyListenerKind {
    pub(crate) async fn stop(self) {
        match self {
            HotkeyListenerKind::Module { cancel, task } => {
                cancel.cancel();
                let _ = task.await;
            }
            #[cfg(any(target_os = "macos", test))]
            HotkeyListenerKind::Globe { cancel, task } => {
                cancel.cancel();
                let _ = task.await;
            }
        }
    }
}

/// Start the appropriate hotkey listener for the current platform and key.
///
/// On macOS, the Fn/Globe key is handled by the Swift-based globe listener
/// inside the TinyComputer module which monitors `NSEvent.flagsChanged`. All other
/// keys return an error on macOS: rdev's CGEventTap callback calls
/// `TSMGetInputSourceProperty` off the main thread; macOS 26 enforces
/// `dispatch_assert_queue(main_queue)` inside that API and kills the process
/// with `EXC_BREAKPOINT` (`dispatch_assert_queue_fail`). Configure
/// `hotkey = "fn"` to avoid this. (#2677)
pub(crate) async fn start_hotkey_listener(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    hotkey_str: &str,
    mode: hotkey::ActivationMode,
    server_cancel: &CancellationToken,
) -> Result<
    (
        HotkeyListenerKind,
        tokio::sync::mpsc::Receiver<hotkey::HotkeyEvent>,
    ),
    String,
> {
    #[cfg(target_os = "macos")]
    {
        if hotkey_str.trim().eq_ignore_ascii_case("fn") {
            return start_globe_hotkey_listener(ctx, config, mode, server_cancel).await;
        }
        // rdev calls TSMGetInputSourceProperty off the main thread; macOS 26
        // enforces main-queue-only access and crashes the process. Only the
        // Fn/Globe key is safe via the Swift globe listener. (#2677)
        Err(format!(
            "voice server hotkey '{}' is not supported on macOS — \
             only 'fn' (Fn/Globe key) is safe. rdev calls \
             TSMGetInputSourceProperty off the main thread on macOS 26, \
             causing EXC_BREAKPOINT. Set hotkey = \"fn\" in voice config \
             (issue #2677).",
            hotkey_str
        ))
    }

    // Non-macOS: TinyVoice owns its native listener and cleanup worker.
    #[cfg(not(target_os = "macos"))]
    {
        start_module_hotkey_listener(ctx, config, hotkey_str, mode, server_cancel).await
    }
}

#[cfg(not(target_os = "macos"))]
async fn start_module_hotkey_listener(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    hotkey_str: &str,
    mode: hotkey::ActivationMode,
    server_cancel: &CancellationToken,
) -> Result<
    (
        HotkeyListenerKind,
        tokio::sync::mpsc::Receiver<hotkey::HotkeyEvent>,
    ),
    String,
> {
    use crate::modules::voice;
    use std::time::Duration;

    let handle = voice::reserve_hotkey(
        config,
        hotkey::HotkeyRequest {
            key: hotkey_str.to_string(),
            mode,
            source: hotkey::HotkeySource::Native,
        },
    )
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| format!("hotkey reservation failed: {error:?}"))?;

    let task_cancel = server_cancel.child_token();
    let worker_cancel = task_cancel.clone();
    let task_config = config.clone();
    let (events_tx, events_rx) = tokio::sync::mpsc::channel(65);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let worker_handle = handle.clone();
    let task = ctx.spawn(async move {
        let generation = loop {
            if worker_cancel.is_cancelled() || events_tx.is_closed() {
                break None;
            }
            match voice::start_hotkey(&task_config, worker_handle.clone()).await {
                Ok(Ok(status)) if status.state == hotkey::HotkeyState::Running => {
                    break status.generation;
                }
                Ok(Ok(_)) => break None,
                Ok(Err(error)) => {
                    let _ = ready_tx.send(Err(format!("hotkey startup failed: {error:?}")));
                    close_module_hotkey(&task_config, worker_handle).await;
                    return;
                }
                Err(error) => {
                    tokio::select! {
                        _ = worker_cancel.cancelled() => break None,
                        _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                    }
                    log::debug!("{LOG_PREFIX} retrying uncertain hotkey start: {error}");
                }
            }
        };
        let Some(generation) = generation else {
            let _ = ready_tx.send(Err("hotkey startup canceled".into()));
            close_module_hotkey(&task_config, worker_handle).await;
            return;
        };
        if ready_tx.send(Ok(())).is_err() {
            close_module_hotkey(&task_config, worker_handle).await;
            return;
        }

        let mut delivery = HotkeyDelivery::new(generation);
        let mut interval = tokio::time::interval(Duration::from_millis(25));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = worker_cancel.cancelled() => break,
                _ = events_tx.closed() => break,
                _ = interval.tick() => {}
            }
            // A batch is capped at 64 events. Reserve one slot for the
            // inactive edge delivered when the module reports a reset.
            if events_tx.capacity() != 65 {
                continue;
            }
            let batch = match voice::read_hotkey(
                &task_config,
                hotkey::HotkeyReadRequest {
                    handle: worker_handle.clone(),
                    acknowledged_batch: delivery.acknowledged_batch,
                },
            )
            .await
            {
                Ok(Ok(batch)) => batch,
                Ok(Err(error)) => {
                    log::warn!("{LOG_PREFIX} module hotkey read failed: {error:?}");
                    break;
                }
                Err(error) => {
                    log::warn!("{LOG_PREFIX} module hotkey read unavailable: {error}");
                    continue;
                }
            };
            let events = match delivery.accept(&batch) {
                Ok(events) => events,
                Err(()) => {
                    log::warn!("{LOG_PREFIX} invalid TinyVoice hotkey batch continuity");
                    break;
                }
            };
            let mut delivered = true;
            for event in events {
                if events_tx.send(event).await.is_err() {
                    delivered = false;
                    break;
                }
            }
            if !delivered {
                break;
            }
        }
        close_module_hotkey(&task_config, worker_handle).await;
    });

    tokio::select! {
        result = ready_rx => result.map_err(|_| "TinyVoice hotkey startup task ended".to_string())??,
        _ = server_cancel.cancelled() => return Err("hotkey startup canceled".into()),
    }
    Ok((
        HotkeyListenerKind::Module {
            cancel: task_cancel,
            task,
        },
        events_rx,
    ))
}

#[cfg(not(target_os = "macos"))]
async fn close_module_hotkey(config: &crate::config::Config, handle: hotkey::HotkeyHandle) {
    loop {
        match crate::modules::voice::stop_hotkey(config, handle.clone()).await {
            Ok(Ok(status)) if status.state == hotkey::HotkeyState::Stopped => return,
            Ok(Ok(status)) => {
                log::warn!(
                    "{LOG_PREFIX} hotkey cleanup still active: {:?}",
                    status.state
                )
            }
            Ok(Err(hotkey::HotkeyError::UnknownHandle)) => return,
            Ok(Err(error)) => log::warn!("{LOG_PREFIX} hotkey cleanup pending: {error:?}"),
            Err(error) => log::warn!("{LOG_PREFIX} hotkey cleanup unavailable: {error}"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

/// Validate and translate one replayable module batch before acknowledging it.
/// The queue stays bounded to one batch, so acknowledgment means every event
/// has been admitted to the host's receiver.
#[cfg(not(target_os = "macos"))]
struct HotkeyDelivery {
    generation: u64,
    acknowledged_batch: Option<u64>,
    active: bool,
    last_sequence: Option<u64>,
}

#[cfg(not(target_os = "macos"))]
impl HotkeyDelivery {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            acknowledged_batch: None,
            active: false,
            last_sequence: None,
        }
    }

    fn accept(&mut self, batch: &hotkey::HotkeyBatch) -> Result<Vec<hotkey::HotkeyEvent>, ()> {
        if batch.generation != self.generation
            || batch.events.len() > 64
            || batch.batch != self.acknowledged_batch.unwrap_or(0).saturating_add(1)
        {
            return Err(());
        }

        let mut active = self.active;
        let mut last_sequence = self.last_sequence;
        let mut output = Vec::with_capacity(batch.events.len() + usize::from(batch.reset));
        if batch.reset {
            if active {
                output.push(hotkey::HotkeyEvent::Released);
            }
            active = false;
            last_sequence = None;
        }
        for event in &batch.events {
            if event.generation != self.generation
                || last_sequence.is_some_and(|last| event.sequence <= last)
            {
                return Err(());
            }
            active = event.event == hotkey::HotkeyEvent::Pressed;
            last_sequence = Some(event.sequence);
            output.push(event.event);
        }
        if active != batch.active {
            return Err(());
        }

        self.acknowledged_batch = Some(batch.batch);
        self.active = active;
        self.last_sequence = last_sequence;
        Ok(output)
    }
}

/// Start an owned task before awaiting the module, so cancellation during startup
/// still leaves an owner to recover a lost start reply and close the native lease.
#[cfg(target_os = "macos")]
async fn start_globe_hotkey_listener(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    mode: hotkey::ActivationMode,
    server_cancel: &CancellationToken,
) -> Result<
    (
        HotkeyListenerKind,
        tokio::sync::mpsc::Receiver<hotkey::HotkeyEvent>,
    ),
    String,
> {
    start_globe_with_client(
        ctx,
        crate::modules::accessibility::AccessibilityClient::new(config),
        mode,
        server_cancel,
    )
    .await
}

#[cfg(any(target_os = "macos", test))]
async fn start_globe_with_client(
    ctx: &crate::core::runtime::DispatchCtx,
    client: crate::modules::accessibility::AccessibilityClient,
    mode: hotkey::ActivationMode,
    server_cancel: &CancellationToken,
) -> Result<
    (
        HotkeyListenerKind,
        tokio::sync::mpsc::Receiver<hotkey::HotkeyEvent>,
    ),
    String,
> {
    static ADMISSION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let admission = tokio::time::timeout(Duration::from_secs(15), ADMISSION.lock())
        .await
        .map_err(|_| "previous Globe listener cleanup is still pending".to_string())?;
    let (tx, rx) = tokio::sync::mpsc::channel(65);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let cancel = server_cancel.child_token();
    let task_cancel = cancel.clone();
    let task = ctx.spawn(async move {
        let _admission = admission;
        let started = match client.globe_start().await {
            Ok(started) => started,
            Err(error) => {
                let recover = matches!(
                    error,
                    crate::modules::accessibility::AccessibilityError::Bus(
                        tinybus::module::ModuleCallError::TransportFailed
                            | tinybus::module::ModuleCallError::ModuleFault
                    )
                );
                let _ = ready_tx.send(Err(error.to_string()));
                if !recover {
                    return;
                }
                // Start is idempotent. Recover its lease before cleanup rather
                // than abandoning a native start whose reply was lost.
                loop {
                    match client.globe_start().await {
                        Ok(started) => {
                            close_globe(&client, started.handle).await;
                            return;
                        }
                        Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
                    }
                }
            }
        };
        let handle = started.handle;
        if !started.status.running {
            let _ = ready_tx.send(Err(started
                .status
                .last_error
                .unwrap_or_else(|| "Globe listener did not start".into())));
            close_globe(&client, handle).await;
            return;
        }
        if ready_tx.send(Ok(())).is_err() {
            close_globe(&client, handle).await;
            return;
        }
        let mut acknowledged_batch = None;
        let mut activation = GlobeActivation::default();
        let mut interval = tokio::time::interval(Duration::from_millis(50));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = task_cancel.cancelled() => break,
                _ = tx.closed() => break,
                _ = interval.tick() => {}
            }
            // One bounded module batch at a time. Wait for the host to consume
            // its queue before accepting another batch (at most 65 events).
            if tx.capacity() != 65 {
                continue;
            }
            let batch = match client
                .globe_read(crate::modules::accessibility::GlobeRead {
                    handle: handle.clone(),
                    acknowledged_batch,
                })
                .await
            {
                Ok(batch) => batch,
                Err(error) => {
                    warn!("{LOG_PREFIX} Globe read failed: {error}");
                    break;
                }
            };
            if batch.handle != handle
                || batch.events.len() > 64
                || batch.batch != acknowledged_batch.unwrap_or(0).saturating_add(1)
            {
                warn!("{LOG_PREFIX} invalid Globe batch");
                break;
            }
            if !batch.status.running {
                break;
            }
            if batch.overflow {
                if let Some(event) = activation.reset() {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
            }
            let mut delivered = true;
            for event in batch.events {
                if let Some(event) = activation.event(event, mode) {
                    if tx.send(event).await.is_err() {
                        delivered = false;
                        break;
                    }
                }
            }
            if !delivered {
                break;
            }
            acknowledged_batch = Some(batch.batch);
        }
        close_globe(&client, handle).await;
    });
    // Dropping the startup future cancels the task's receiver; ready_tx then
    // fails and the task closes its lease before releasing admission.
    ready_rx
        .await
        .map_err(|_| "Globe startup task failed".to_string())??;
    Ok((HotkeyListenerKind::Globe { cancel, task }, rx))
}

/// Keep the admission guard while close fails; a successor cannot adopt a
/// listener with uncertain native cleanup. An already-retired lease is closed.
#[cfg(any(target_os = "macos", test))]
async fn close_globe(
    client: &crate::modules::accessibility::AccessibilityClient,
    handle: crate::modules::accessibility::GlobeHandle,
) {
    loop {
        match client.globe_stop(handle.clone()).await {
            Ok(_) => return,
            Err(crate::modules::accessibility::AccessibilityError::Operation(error))
                if error.code == "UNKNOWN_LISTENER" =>
            {
                return
            }
            Err(error) => warn!("{LOG_PREFIX} Globe cleanup pending: {error}"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Host activation policy; an overflow requires a physical release before rearming.
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct GlobeActivation {
    active: bool,
    wait_release: bool,
}

#[cfg(any(target_os = "macos", test))]
impl GlobeActivation {
    fn reset(&mut self) -> Option<hotkey::HotkeyEvent> {
        self.wait_release = true;
        std::mem::take(&mut self.active).then_some(hotkey::HotkeyEvent::Released)
    }
    fn event(
        &mut self,
        event: tinycomputer_bus::accessibility::GlobeEvent,
        mode: hotkey::ActivationMode,
    ) -> Option<hotkey::HotkeyEvent> {
        use tinycomputer_bus::accessibility::GlobeEvent::{Down, Up};
        if self.wait_release {
            if event == Up {
                self.wait_release = false;
            }
            return None;
        }
        match (event, mode) {
            (Down, hotkey::ActivationMode::Tap) => {
                self.active = !self.active;
                Some(if self.active {
                    hotkey::HotkeyEvent::Pressed
                } else {
                    hotkey::HotkeyEvent::Released
                })
            }
            (Down, hotkey::ActivationMode::Push) if !self.active => {
                self.active = true;
                Some(hotkey::HotkeyEvent::Pressed)
            }
            (Up, hotkey::ActivationMode::Push) if self.active => {
                self.active = false;
                Some(hotkey::HotkeyEvent::Released)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "hotkey_listener_tests.rs"]
mod tests;
