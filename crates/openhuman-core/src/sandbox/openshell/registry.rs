//! [`BoxRegistry`]: one live box per tenant, created on first use, reused,
//! and closed when idle, released, or no longer answering.
//!
//! State lives in one map behind a `std` mutex that is never held across an
//! `.await`. Each tenant's entry counts the commands in flight on it and when
//! it was last used; its [`Slot`] holds the box id and the gate that makes
//! creation single-flight. A slot taken out of the map (released, shut down,
//! reaped) is marked retired, so a command still running on it can never
//! create a box nobody tracks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError, Weak};
use std::time::Duration;

use tokio::time::Instant;

use super::{BoxDriver, BoxError, BoxExec, BoxOutput, BoxRequest, BoxSettings, RegistryLimits};
use crate::core::runtime::Tenant;
use crate::sandbox::types::SandboxExecResult;

/// The working directory commands run in: the box's own workspace.
pub(crate) const BOX_WORKDIR: &str = "/sandbox";

/// Seconds `timeout` waits after its signal before it kills the command.
const KILL_AFTER_SECS: &str = "5";

/// How long past the command's deadline the host waits for the box to answer
/// before it gives the box up. Covers the kill grace above plus the reply.
pub(crate) const BACKSTOP_MARGIN: Duration = Duration::from_secs(10);

/// The exit status `timeout` reports for a command it stopped.
const TIMED_OUT_EXIT: i32 = 124;

/// The longest the background reaper waits between passes.
pub(crate) const REAP_EVERY_AT_MOST: Duration = Duration::from_secs(30);

/// One live box per [`Tenant`], over a [`BoxDriver`].
pub struct BoxRegistry<D: BoxDriver> {
    driver: Arc<D>,
    limits: RegistryLimits,
    entries: Mutex<HashMap<Tenant, Entry>>,
    reaper: Once,
}

struct Entry {
    slot: Arc<Slot>,
    in_flight: usize,
    last_used: Instant,
}

#[derive(Default)]
struct Slot {
    /// Held while the box is created, so concurrent first commands make one.
    create_gate: tokio::sync::Mutex<()>,
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    resource: Option<String>,
    retired: bool,
}

impl Slot {
    fn state(&self) -> MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn current(&self) -> Result<Option<String>, BoxError> {
        let state = self.state();
        if state.retired {
            return Err(BoxError::Released);
        }
        Ok(state.resource.clone())
    }

    /// Record a new box; `false` when the slot was retired meanwhile.
    fn store(&self, resource: &str) -> bool {
        let mut state = self.state();
        if state.retired {
            return false;
        }
        state.resource = Some(resource.to_owned());
        true
    }

    /// Drop `resource` if it is still the slot's box.
    fn forget(&self, resource: &str) {
        let mut state = self.state();
        if state.resource.as_deref() == Some(resource) {
            state.resource = None;
        }
    }

    /// Retire the slot and hand back its box to close.
    fn retire(&self) -> Option<String> {
        let mut state = self.state();
        state.retired = true;
        state.resource.take()
    }
}

/// A command's hold on its tenant's entry; dropping it (finished or
/// abandoned) ends the hold and touches the entry.
struct Lease<'a, D: BoxDriver> {
    registry: &'a BoxRegistry<D>,
    tenant: Tenant,
    slot: Arc<Slot>,
}

impl<D: BoxDriver> Drop for Lease<'_, D> {
    fn drop(&mut self) {
        self.registry.check_in(&self.tenant, &self.slot);
    }
}

enum Run {
    Finished(BoxOutput),
    BackstopFired,
}

impl<D: BoxDriver> BoxRegistry<D> {
    /// A registry with no boxes yet. Call [`Self::start_reaper`] to close
    /// idle boxes in the background.
    pub fn new(driver: Arc<D>, limits: RegistryLimits) -> Arc<Self> {
        Arc::new(Self {
            driver,
            limits,
            entries: Mutex::new(HashMap::new()),
            reaper: Once::new(),
        })
    }

    /// How many tenants hold a box (or are creating one).
    pub fn live(&self) -> usize {
        self.entries().len()
    }

    /// Run `request` in `tenant`'s box, creating the box first if it has none.
    ///
    /// A box the driver reports gone is replaced once and the command retried.
    /// A box that does not answer by the command's deadline plus
    /// [`BACKSTOP_MARGIN`] is closed, and the command reported as timed out.
    ///
    /// # Errors
    ///
    /// [`BoxError::AtCapacity`] when a new box would exceed `max_live`, and
    /// any creation or execution failure from the driver.
    pub async fn exec(
        &self,
        tenant: &Tenant,
        settings: &BoxSettings,
        request: &BoxRequest,
    ) -> Result<SandboxExecResult, BoxError> {
        let lease = self.check_out(tenant).await?;
        let mut replaced = false;
        loop {
            let resource = self.resource_for(&lease.slot, settings).await?;
            match self.run(&resource, request).await {
                Ok(Run::Finished(output)) => return Ok(finished(output)),
                Ok(Run::BackstopFired) => {
                    tracing::warn!(
                        timeout_secs = whole_seconds(request.timeout),
                        "[sandbox:openshell] box did not answer by the deadline; closing it"
                    );
                    lease.slot.forget(&resource);
                    self.close_quietly(&resource).await;
                    return Ok(backstop_timed_out(request.timeout));
                }
                Err(BoxError::UnknownResource) => {
                    // "Gone" is inferred after a failed exec, so the box may
                    // still exist on the gateway: close it rather than leak it.
                    lease.slot.forget(&resource);
                    self.close_quietly(&resource).await;
                    if replaced {
                        return Err(BoxError::UnknownResource);
                    }
                    tracing::warn!("[sandbox:openshell] box is gone; retrying in a new box");
                    replaced = true;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Close every box idle since `now - idle_ttl` or earlier with no command
    /// in flight; returns how many were closed.
    pub async fn reap(&self, now: Instant) -> usize {
        let idle = self.take_idle(&mut self.entries(), now);
        let closed = idle.len();
        for resource in idle {
            self.close_quietly(&resource).await;
        }
        if closed > 0 {
            tracing::debug!(closed, "[sandbox:openshell] reaped idle boxes");
        }
        closed
    }

    /// Close `tenant`'s box, if it has one. A command still running in it
    /// fails rather than starting another box.
    pub async fn release(&self, tenant: &Tenant) {
        let resource = self
            .entries()
            .remove(tenant)
            .and_then(|entry| entry.slot.retire());
        if let Some(resource) = resource {
            self.close_quietly(&resource).await;
        }
    }

    /// Close every box.
    pub async fn shutdown(&self) {
        let resources: Vec<String> = self
            .entries()
            .drain()
            .filter_map(|(_, entry)| entry.slot.retire())
            .collect();
        for resource in resources {
            self.close_quietly(&resource).await;
        }
    }

    /// Start the background reaper, once; it stops when the registry drops.
    pub fn start_reaper(self: &Arc<Self>) {
        self.reaper.call_once(|| {
            let registry: Weak<Self> = Arc::downgrade(self);
            let every = reap_interval(self.limits.idle_ttl);
            // A process-wide sweeper: it closes boxes by id and acts under no
            // tenant's context, so it carries none.
            tokio::spawn(async move {
                let mut ticks = tokio::time::interval(every);
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticks.tick().await;
                    let Some(registry) = registry.upgrade() else {
                        return;
                    };
                    registry.reap(Instant::now()).await;
                }
            });
        });
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<Tenant, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hold `tenant`'s entry, making one when it has none and there is room.
    async fn check_out(&self, tenant: &Tenant) -> Result<Lease<'_, D>, BoxError> {
        let (held, reaped) = self.hold(tenant, Instant::now());
        for resource in reaped {
            self.close_quietly(&resource).await;
        }
        let Some(slot) = held else {
            tracing::warn!(
                max_live = self.limits.max_live,
                "[sandbox:openshell] refusing a new box: at capacity"
            );
            return Err(BoxError::AtCapacity);
        };
        Ok(Lease {
            registry: self,
            tenant: tenant.clone(),
            slot,
        })
    }

    /// Count a command against `tenant`'s entry, making the entry when there
    /// is room (after reaping idle boxes if there is not). Returns the slot,
    /// or `None` at capacity, and the reaped boxes to close.
    fn hold(&self, tenant: &Tenant, now: Instant) -> (Option<Arc<Slot>>, Vec<String>) {
        let mut entries = self.entries();
        let mut reaped = Vec::new();
        if !entries.contains_key(tenant) && entries.len() >= self.limits.max_live {
            reaped = self.take_idle(&mut entries, now);
            if entries.len() >= self.limits.max_live {
                return (None, reaped);
            }
        }
        let entry = entries.entry(tenant.clone()).or_insert_with(|| Entry {
            slot: Arc::new(Slot::default()),
            in_flight: 0,
            last_used: now,
        });
        entry.in_flight += 1;
        entry.last_used = now;
        (Some(Arc::clone(&entry.slot)), reaped)
    }

    /// End a command's hold; an entry left with no box and no command goes.
    fn check_in(&self, tenant: &Tenant, slot: &Arc<Slot>) {
        let mut entries = self.entries();
        let Some(entry) = entries.get_mut(tenant) else {
            return;
        };
        if !Arc::ptr_eq(&entry.slot, slot) {
            return;
        }
        entry.in_flight = entry.in_flight.saturating_sub(1);
        entry.last_used = Instant::now();
        if entry.in_flight == 0 && entry.slot.state().resource.is_none() {
            entries.remove(tenant);
        }
    }

    /// Remove idle entries and return their boxes to close.
    fn take_idle(&self, entries: &mut HashMap<Tenant, Entry>, now: Instant) -> Vec<String> {
        let ttl = self.limits.idle_ttl;
        let mut resources = Vec::new();
        entries.retain(|_, entry| {
            let idle =
                entry.in_flight == 0 && now.saturating_duration_since(entry.last_used) >= ttl;
            if idle {
                resources.extend(entry.slot.retire());
            }
            !idle
        });
        resources
    }

    /// The slot's box, created now if it has none.
    async fn resource_for(&self, slot: &Slot, settings: &BoxSettings) -> Result<String, BoxError> {
        if let Some(resource) = slot.current()? {
            return Ok(resource);
        }
        let _creating = slot.create_gate.lock().await;
        if let Some(resource) = slot.current()? {
            return Ok(resource);
        }
        tracing::debug!(image = %settings.image, "[sandbox:openshell] creating a box");
        let resource = self.driver.create(settings).await?;
        if !slot.store(&resource) {
            self.close_quietly(&resource).await;
            return Err(BoxError::Released);
        }
        tracing::debug!(resource = %resource, "[sandbox:openshell] box ready");
        Ok(resource)
    }

    async fn run(&self, resource: &str, request: &BoxRequest) -> Result<Run, BoxError> {
        let secs = whole_seconds(request.timeout);
        let deadline = Duration::from_secs(secs).saturating_add(BACKSTOP_MARGIN);
        let exec = BoxExec {
            argv: timeout_argv(secs, &request.command),
            cwd: BOX_WORKDIR.to_owned(),
            env: request.env.clone(),
            deadline,
        };
        match tokio::time::timeout(deadline, self.driver.exec(resource, &exec)).await {
            Ok(output) => output.map(Run::Finished),
            Err(_) => Ok(Run::BackstopFired),
        }
    }

    async fn close_quietly(&self, resource: &str) {
        if let Err(error) = self.driver.close(resource).await {
            tracing::warn!(
                resource = %resource,
                error = %error,
                "[sandbox:openshell] closing a box failed"
            );
        }
    }
}

/// `timeout -k 5 <secs> sh -c <command>`: the box stops the command itself.
fn timeout_argv(secs: u64, command: &str) -> Vec<String> {
    [
        "timeout",
        "-k",
        KILL_AFTER_SECS,
        &secs.to_string(),
        "sh",
        "-c",
        command,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// `timeout` counts whole seconds: round up, and never ask for none.
pub(crate) fn whole_seconds(timeout: Duration) -> u64 {
    let secs = timeout.as_secs() + u64::from(timeout.subsec_nanos() > 0);
    secs.max(1)
}

/// Half the TTL, at most [`REAP_EVERY_AT_MOST`], at least a second.
pub(crate) fn reap_interval(idle_ttl: Duration) -> Duration {
    (idle_ttl / 2)
        .min(REAP_EVERY_AT_MOST)
        .max(Duration::from_secs(1))
}

/// A command's output as a sandbox result; exit 124 is `timeout` stopping it.
pub(crate) fn finished(output: BoxOutput) -> SandboxExecResult {
    SandboxExecResult {
        exit_code: output.exit_code,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        timed_out: output.exit_code == TIMED_OUT_EXIT,
    }
}

fn backstop_timed_out(timeout: Duration) -> SandboxExecResult {
    SandboxExecResult {
        exit_code: TIMED_OUT_EXIT,
        stdout: String::new(),
        stderr: format!(
            "the command did not finish within {}s and its sandbox was discarded",
            whole_seconds(timeout)
        ),
        timed_out: true,
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
