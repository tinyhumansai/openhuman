//! [`BoxRegistry`]: one live box per tenant, created on first use, reused,
//! and let go of when idle, released, cancelled, or no longer answering.
//!
//! Boxes are kept per [`OwnerScope`] and tenant: the scope (the deployment's
//! namespace and the active workspace, from the call's [`BoxSettings`]) keeps
//! one host's users apart from another's, and a signed-in user's from the
//! next one's, even when their tenants are equal.
//!
//! A registry over a backend that keeps boxes for their owners
//! ([`RegistryLimits::retain`], agent37) creates each box with its tenant's
//! [`owner_key`], and letting go of a box only forgets it
//! ([`BoxDriver::forget`]): the backend sleeps it, and the next command finds
//! the same box again under the same key. Because forgetting costs nothing, a
//! full retaining registry also forgets its least recently used idle box to
//! make room. Every other registry (OpenShell) closes boxes it lets go of.
//!
//! State lives in one map behind a `std` mutex that is never held across an
//! `.await`. Each tenant's entry counts the commands in flight on it and when
//! it was last used; its [`Slot`] holds the box id and the gate that makes
//! creation single-flight. A slot taken out of the map (released, shut down,
//! reaped) is marked retired, so a command still running on it can never
//! create a box nobody tracks.
//!
//! Every await in [`BoxRegistry::exec`] is a cancellation point, and each
//! leaves the registry consistent: the command's hold on its entry is taken
//! before the first await and dropped with the future, and a command dropped
//! while it runs in a box lets go of that box on a detached task (closing it,
//! or forgetting a retained one), which stops the command there rather than
//! leaving it to run on with the next command queued behind it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError, Weak};
use std::time::Duration;

use tokio::time::Instant;

use sha2::{Digest, Sha256};

use super::cleanup::{spawn_detached, Cleanups, Detached};
use super::{
    BoxDriver, BoxError, BoxExec, BoxOutput, BoxRequest, BoxSettings, OwnerScope, RegistryLimits,
    Retention,
};
use crate::core::runtime::Tenant;
use crate::sandbox::types::SandboxExecResult;

/// Prefix of every [`owner_key`], naming who chose the key.
const OWNER_KEY_PREFIX: &str = "oh-";

/// Domain separation for the [`owner_key`] hash, versioned so a change to the
/// derivation can never collide with keys already in use. v2 folds in the
/// [`OwnerScope`]; boxes kept under v1 keys are never adopted again and expire.
const OWNER_KEY_DOMAIN: &[u8] = b"openhuman.sandbox.box-owner.v2\0";

/// Seconds `timeout` waits after its signal before it kills the command.
const KILL_AFTER_SECS: &str = "5";

/// How long past the command's deadline the host waits for the box to answer
/// before it gives the box up. Covers the kill grace above plus the reply.
pub(crate) const BACKSTOP_MARGIN: Duration = Duration::from_secs(10);

/// The exit status `timeout` reports for a command it stopped.
const TIMED_OUT_EXIT: i32 = 124;

/// The longest the background reaper waits between passes.
pub(crate) const REAP_EVERY_AT_MOST: Duration = Duration::from_secs(30);

/// Who an entry's box is for: the tenant within its scope.
pub(super) type Owner = (OwnerScope, Tenant);

/// One live box per [`Tenant`] and [`OwnerScope`], over a [`BoxDriver`].
pub struct BoxRegistry<D: BoxDriver> {
    driver: Arc<D>,
    limits: RegistryLimits,
    entries: Mutex<HashMap<Owner, Entry>>,
    cleanups: Arc<Cleanups>,
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
    /// Held while a command runs in the box, so commands on one box run one
    /// at a time. A caller waiting here has armed nothing, so giving up while
    /// it waits never closes the box under the command that holds the gate.
    exec_gate: tokio::sync::Mutex<()>,
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

    /// Give up `resource` as the slot's box, returning its cleanup (already
    /// pending). `None` when the slot no longer owns it: it was retired, so
    /// the retirement took the box and its cleanup, or it was given up
    /// already. A stale command must not then close a replacement's box.
    fn surrender(
        &self,
        resource: &str,
        cleanups: &Arc<Cleanups>,
        owner: &Owner,
    ) -> Option<Detached> {
        let mut state = self.state();
        if state.resource.as_deref() != Some(resource) {
            return None;
        }
        state.resource = None;
        Some(cleanups.detach(owner.clone(), resource.to_owned()))
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
    owner: Owner,
    slot: Arc<Slot>,
}

impl<D: BoxDriver> Drop for Lease<'_, D> {
    fn drop(&mut self) {
        self.registry.check_in(&self.owner, &self.slot);
    }
}

/// A command running in a box. Dropped before [`Self::finish`] (the caller
/// gave up), it takes the box from its slot and lets go of it on a detached
/// task, since `Drop` cannot await: closing a box stops what runs in it, and
/// forgetting a retained one drops the host's handle and its exec with it.
struct Running<D: BoxDriver> {
    driver: Arc<D>,
    retained: bool,
    cleanups: Arc<Cleanups>,
    owner: Owner,
    /// Captured while the command is armed: a cancelled command may be dropped
    /// outside any runtime context, and its box must still be let go of.
    runtime: tokio::runtime::Handle,
    slot: Arc<Slot>,
    resource: Option<String>,
}

impl<D: BoxDriver> Running<D> {
    /// The command answered (or failed); its box stays with the slot.
    fn finish(mut self) {
        self.resource = None;
    }
}

impl<D: BoxDriver> Drop for Running<D> {
    fn drop(&mut self) {
        let Some(resource) = self.resource.take() else {
            return;
        };
        tracing::debug!(
            resource = %resource,
            retained = self.retained,
            "[sandbox:box] command cancelled; letting go of its box"
        );
        let Some(detached) = self.slot.surrender(&resource, &self.cleanups, &self.owner) else {
            tracing::debug!("[sandbox:box] the box was already let go of; nothing to clean up");
            return;
        };
        // The box is let go of on a task of its own; the handle is not needed.
        drop(spawn_detached(
            &self.driver,
            self.retained,
            &self.runtime,
            detached,
        ));
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
            cleanups: Arc::new(Cleanups::default()),
            reaper: Once::new(),
        })
    }

    /// How many tenants hold a box (or are creating one).
    pub fn live(&self) -> usize {
        self.entries().len()
    }

    /// The driver this registry creates, runs and lets go of boxes with.
    pub fn driver(&self) -> &Arc<D> {
        &self.driver
    }

    /// Run `request` in `tenant`'s box within `settings.owner_scope`, creating
    /// the box first if it has none.
    ///
    /// A box the driver reports gone is replaced once and the command retried.
    /// A box that does not answer by the command's deadline plus
    /// [`BACKSTOP_MARGIN`] is closed, and the command reported as timed out.
    /// Dropping the returned future while the command runs lets go of its box
    /// (closed, or forgotten when retained), which stops the command.
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
        let lease = self
            .check_out((settings.owner_scope.clone(), tenant.clone()))
            .await?;
        // One command at a time per box: a command queued behind another arms
        // nothing until it runs, so cancelling it cannot close the box the
        // running command is using.
        let slot = Arc::clone(&lease.slot);
        let _turn = slot.exec_gate.lock().await;
        let mut replaced = false;
        loop {
            let resource = self.resource_for(&lease.owner, &slot, settings).await?;
            let running = Running {
                driver: Arc::clone(&self.driver),
                retained: self.limits.retain.is_some(),
                cleanups: Arc::clone(&self.cleanups),
                owner: lease.owner.clone(),
                runtime: tokio::runtime::Handle::current(),
                slot: Arc::clone(&slot),
                resource: Some(resource.clone()),
            };
            let outcome = self.run(&resource, settings, request).await;
            running.finish();
            match outcome {
                Ok(Run::Finished(output)) => return Ok(finished(output)),
                Ok(Run::BackstopFired) => {
                    tracing::warn!(
                        timeout_secs = whole_seconds(request.timeout),
                        "[sandbox:box] box did not answer by the deadline; letting it go"
                    );
                    if let Some(detached) = slot.surrender(&resource, &self.cleanups, &lease.owner)
                    {
                        self.let_go_all(vec![detached]).await;
                    }
                    return Ok(backstop_timed_out(request.timeout));
                }
                Err(BoxError::UnknownResource) => {
                    // "Gone" is inferred after a failed exec, so the box may
                    // still exist on the gateway: let go of it rather than
                    // leak it (a retained box is only forgotten).
                    if let Some(detached) = slot.surrender(&resource, &self.cleanups, &lease.owner)
                    {
                        self.let_go_all(vec![detached]).await;
                    }
                    if replaced {
                        return Err(BoxError::UnknownResource);
                    }
                    tracing::warn!("[sandbox:box] box is gone; retrying in a new box");
                    replaced = true;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Let go of every box idle since `now - idle_ttl` or earlier with no
    /// command in flight (closed, or forgotten when retained); returns how
    /// many.
    pub async fn reap(&self, now: Instant) -> usize {
        let idle = self.take_idle(&mut self.entries(), now);
        let reaped = idle.len();
        self.let_go_all(idle).await;
        if reaped > 0 {
            tracing::debug!(
                reaped,
                retained = self.limits.retain.is_some(),
                "[sandbox:box] reaped idle boxes"
            );
        }
        reaped
    }

    /// Let go of `tenant`'s boxes, in every scope (closed, or forgotten when
    /// retained). A command still running in one fails rather than starting
    /// another box.
    pub async fn release(&self, tenant: &Tenant) {
        let mut detached = Vec::new();
        self.entries().retain(|(scope, owner), entry| {
            if owner != tenant {
                return true;
            }
            let key = (scope.clone(), owner.clone());
            detached.extend(
                entry
                    .slot
                    .retire()
                    .map(|resource| self.cleanups.detach(key.clone(), resource)),
            );
            false
        });
        self.let_go_all(detached).await;
        // Cleanups begun earlier for this tenant (a command that gave up) are
        // waited for too, so none outlives the release.
        self.cleanups
            .settle_matching(|(_, owner)| owner == tenant)
            .await;
    }

    /// Let go of every box: closed, or forgotten when retained.
    pub async fn shutdown(&self) {
        let detached: Vec<Detached> = self
            .entries()
            .drain()
            .filter_map(|(owner, entry)| {
                entry
                    .slot
                    .retire()
                    .map(|resource| self.cleanups.detach(owner, resource))
            })
            .collect();
        self.let_go_all(detached).await;
        // Cleanups already pending (a command that gave up) finish before
        // shutdown returns, so no box is left open once the runtime ends.
        self.cleanups.settle_matching(|_| true).await;
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

    fn entries(&self) -> MutexGuard<'_, HashMap<Owner, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hold `owner`'s entry, making one when it has none and there is room.
    /// The hold is a [`Lease`] before anything is awaited, so a caller that
    /// gives up while reaped boxes are let go of still ends it.
    async fn check_out(&self, owner: Owner) -> Result<Lease<'_, D>, BoxError> {
        let (held, reaped) = self.hold(&owner, Instant::now());
        // Every reaped box is scheduled before anything is awaited, so
        // cancelling this call cannot leave one unclosed.
        let cleanups = self.schedule(reaped);
        let lease = held.map(|slot| Lease {
            registry: self,
            owner,
            slot,
        });
        for handle in cleanups {
            self.log_cleanup(handle.await);
        }
        lease.ok_or_else(|| {
            tracing::warn!(
                max_live = self.limits.max_live,
                "[sandbox:box] refusing a new box: at capacity"
            );
            BoxError::AtCapacity
        })
    }

    /// Count a command against `tenant`'s entry, making the entry when there
    /// is room (after reaping idle boxes if there is not). Returns the slot,
    /// or `None` at capacity, and the reaped boxes to close.
    fn hold(&self, owner: &Owner, now: Instant) -> (Option<Arc<Slot>>, Vec<Detached>) {
        let mut entries = self.entries();
        let mut reaped = Vec::new();
        if !entries.contains_key(owner) && entries.len() >= self.limits.max_live {
            reaped = self.take_idle(&mut entries, now);
            if entries.len() >= self.limits.max_live && self.limits.retain.is_some() {
                reaped.extend(take_least_recently_used(&mut entries, &self.cleanups));
            }
            if entries.len() >= self.limits.max_live {
                return (None, reaped);
            }
        }
        let entry = entries.entry(owner.clone()).or_insert_with(|| Entry {
            slot: Arc::new(Slot::default()),
            in_flight: 0,
            last_used: now,
        });
        entry.in_flight += 1;
        entry.last_used = now;
        (Some(Arc::clone(&entry.slot)), reaped)
    }

    /// End a command's hold; an entry left with no box and no command goes.
    fn check_in(&self, owner: &Owner, slot: &Arc<Slot>) {
        let mut entries = self.entries();
        let Some(entry) = entries.get_mut(owner) else {
            return;
        };
        if !Arc::ptr_eq(&entry.slot, slot) {
            return;
        }
        entry.in_flight = entry.in_flight.saturating_sub(1);
        entry.last_used = Instant::now();
        if entry.in_flight == 0 && entry.slot.state().resource.is_none() {
            entries.remove(owner);
        }
    }

    /// Remove idle entries and return their boxes to close.
    fn take_idle(&self, entries: &mut HashMap<Owner, Entry>, now: Instant) -> Vec<Detached> {
        let ttl = self.limits.idle_ttl;
        let mut resources = Vec::new();
        entries.retain(|owner, entry| {
            let idle =
                entry.in_flight == 0 && now.saturating_duration_since(entry.last_used) >= ttl;
            if idle {
                resources.extend(
                    entry
                        .slot
                        .retire()
                        .map(|resource| self.cleanups.detach(owner.clone(), resource)),
                );
            }
            !idle
        });
        resources
    }

    /// The slot's box, created now if it has none. A retaining registry
    /// creates it with `owner`'s retention, so the backend hands back the box
    /// it already keeps for the owner; a refusal is returned as it is, never
    /// retried as a disposable box.
    async fn resource_for(
        &self,
        owner: &Owner,
        slot: &Slot,
        settings: &BoxSettings,
    ) -> Result<String, BoxError> {
        if let Some(resource) = slot.current()? {
            return Ok(resource);
        }
        let _creating = slot.create_gate.lock().await;
        if let Some(resource) = slot.current()? {
            return Ok(resource);
        }
        // A box let go of for this owner may still be closing; a new one made
        // now could adopt the same instance, so wait for it first.
        self.cleanups.settle(owner).await;
        let (scope, tenant) = owner;
        let retention: Option<Retention> = self
            .limits
            .retain
            .map(|policy| policy.for_owner(owner_key(scope, tenant)));
        tracing::debug!(
            image = %settings.image,
            retained = retention.is_some(),
            "[sandbox:box] creating a box"
        );
        let resource = self.driver.create(settings, retention.as_ref()).await?;
        if !slot.store(&resource) {
            let detached = self.cleanups.detach(owner.clone(), resource);
            self.let_go_all(vec![detached]).await;
            return Err(BoxError::Released);
        }
        tracing::debug!(resource = %resource, "[sandbox:box] box ready");
        Ok(resource)
    }

    async fn run(
        &self,
        resource: &str,
        settings: &BoxSettings,
        request: &BoxRequest,
    ) -> Result<Run, BoxError> {
        let secs = whole_seconds(request.timeout);
        let deadline = Duration::from_secs(secs).saturating_add(BACKSTOP_MARGIN);
        let exec = BoxExec {
            argv: timeout_argv(secs, &request.command),
            cwd: settings.workdir.clone(),
            env: request.env.clone(),
            deadline,
        };
        match tokio::time::timeout(deadline, self.driver.exec(resource, &exec)).await {
            Ok(output) => output.map(Run::Finished),
            Err(_) => Ok(Run::BackstopFired),
        }
    }

    /// Let go of a box this registry no longer tracks: forget a retained one
    /// (its backend keeps it, asleep), close any other. A failure is logged.
    /// The cleanup runs on its own task and is awaited here, so a caller
    /// dropped meanwhile still leaves the cleanup to finish.
    /// Spawn the cleanups of detached boxes, each on its own task. Must be
    /// called from within the runtime.
    fn schedule(&self, detached: Vec<Detached>) -> Vec<tokio::task::JoinHandle<()>> {
        let runtime = tokio::runtime::Handle::current();
        let retained = self.limits.retain.is_some();
        detached
            .into_iter()
            .map(|detached| spawn_detached(&self.driver, retained, &runtime, detached))
            .collect()
    }

    /// Let go of detached boxes and wait for each cleanup to finish. They are
    /// all scheduled before the first await, so a caller dropped meanwhile
    /// still leaves every cleanup to run.
    async fn let_go_all(&self, detached: Vec<Detached>) {
        for handle in self.schedule(detached) {
            self.log_cleanup(handle.await);
        }
    }

    fn log_cleanup(&self, outcome: Result<(), tokio::task::JoinError>) {
        if let Err(error) = outcome {
            tracing::warn!(%error, "[sandbox:box] a box cleanup task ended abnormally");
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

/// Remove the least recently used entry with no command in flight and
/// return its box to let go of. Only a retaining registry does this: its box
/// is forgotten, not deleted, so the tenant finds it again next time.
fn take_least_recently_used(
    entries: &mut HashMap<Owner, Entry>,
    cleanups: &Arc<Cleanups>,
) -> Option<Detached> {
    let owner = entries
        .iter()
        .filter(|(_, entry)| entry.in_flight == 0)
        .min_by_key(|(_, entry)| entry.last_used)
        .map(|(owner, _)| owner.clone())?;
    let resource = entries.remove(&owner)?.slot.retire()?;
    Some(cleanups.detach(owner, resource))
}

/// The opaque key a retaining backend keeps `tenant`'s box under within
/// `scope`: `oh-` and the hex SHA-256 of the scope's hash (or its absence)
/// and the whole tenant (profile and agent, each length-prefixed and marked
/// present or absent), so it is stable across restarts, distinct per scope
/// and tenant, never a raw id, and within the backend's key alphabet
/// (`[A-Za-z0-9_-]`, 67 characters).
pub fn owner_key(scope: &OwnerScope, tenant: &Tenant) -> String {
    let mut hash = Sha256::new();
    hash.update(OWNER_KEY_DOMAIN);
    match scope.digest() {
        None => hash.update([0u8]),
        Some(digest) => {
            hash.update([1u8]);
            hash.update(digest);
        }
    }
    for part in [&tenant.profile, &tenant.agent] {
        match part {
            None => hash.update([0u8]),
            Some(value) => {
                hash.update([1u8]);
                hash.update((value.len() as u64).to_le_bytes());
                hash.update(value.as_bytes());
            }
        }
    }
    format!("{OWNER_KEY_PREFIX}{}", hex::encode(hash.finalize()))
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "registry_retain_tests.rs"]
mod retain_tests;
