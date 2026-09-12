//! The active workspace, cached in memory (#5966).
//!
//! [`active_workspace_dir`](super::dirs::active_workspace_dir) resolves the
//! workspace through the loader's own path, which means reading a marker
//! file. That is the right cost for a *decision* — the notification bridge
//! pays it for the handful of workspace-bound events a supervisor tick
//! produces — and the wrong cost for a *stream*. The developer Event Log in
//! [`crate::core::jsonrpc`] has to stamp every domain event the process
//! publishes, and its `tokio_stream` `filter_map` closure is synchronous, so
//! it could not await a disk read even if the cost were acceptable.
//!
//! This module is the cheap answer: a process-global slot holding the last
//! workspace the loader resolved, readable synchronously and without I/O.
//!
//! # The disk stays the source of truth
//!
//! The cache is never authoritative. It is written *through* — every
//! successful resolution publishes its own answer here — and cleared
//! whenever one of the markers that decides the answer is rewritten, so the
//! next reader that can afford a resolve refills it. A stale value is
//! therefore not a thing this can hold: the marker writes and the resolves
//! are the only two ways the answer changes, and both touch this slot.
//!
//! `None` means "not resolved since the last change", not "no workspace".
//! Callers that cannot resolve must treat it as unknown rather than as a
//! mismatch — see the Event Log's handling in `core::jsonrpc`.
//!
//! # Why the env-injectable loader does not publish
//!
//! [`Config::load_or_init_with_env_lookup`](crate::openhuman::config::Config)
//! takes an [`EnvLookup`](super::env::EnvLookup) so tests can exercise the
//! `OPENHUMAN_WORKSPACE` branch without mutating the process environment.
//! Publishing from there would let one test's fixture directory become the
//! whole binary's idea of the active workspace. Only the two `ProcessEnv`
//! entry points publish: `Config::load_or_init` and `active_workspace_dir`.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use once_cell::sync::Lazy;

const LOG_PREFIX: &str = "[config:active-workspace]";

/// The state machine, separated from the process-global slot that holds it.
///
/// The decisions here — when a resolve is a *change*, what invalidation does
/// and does not clear — are the part worth asserting, and asserting them on
/// the global would be unsound: `Config::load_or_init` publishes into that
/// global, and the test binary runs thousands of tests in parallel, many of
/// which load a config. A test that pinned the global would pass alone and
/// fail whenever it happened to interleave with one of those.
#[derive(Default)]
struct ActiveWorkspace {
    /// The last resolved workspace, or `None` when a marker write has
    /// invalidated it and nothing has resolved since.
    current: Option<PathBuf>,
    /// The workspace last announced on the bus.
    ///
    /// Kept separately from `current`, and deliberately **not** cleared by
    /// invalidation. A marker write does not always change the answer —
    /// signing in as the user who is already active rewrites
    /// `active_user.toml` with the same id — and without this the re-resolve
    /// that follows would announce a switch that never happened, putting a
    /// phantom row in the Event Log and making a real switch harder to spot.
    announced: Option<PathBuf>,
    /// Monotonic revision, incremented on every announced transition.
    ///
    /// The bus publish happens *outside* the lock — a subscriber may reach
    /// back into the config layer, and holding the slot across the publish
    /// would deadlock — which leaves a window between committing the state
    /// and emitting the event. Two resolvers can interleave there: A commits
    /// A, B commits B and emits B, then A resumes and emits a stale A. Since
    /// `announced` already equals B by then, no later resolution of B would
    /// correct it, and every client would sit on A permanently.
    ///
    /// The revision closes that: a publisher captures the revision it
    /// created, and emits only if it is still the newest. The stale emit is
    /// dropped rather than reordered, because the newer publisher has
    /// already emitted the right answer. It also goes on the wire, so a
    /// client can discard a snapshot that lost a race to a switch.
    revision: u64,
}

/// One announced transition: what to say, and which revision says it.
struct Transition {
    workspace_dir: PathBuf,
    revision: u64,
}

/// What a resolve left behind.
struct Published {
    /// Revision of the resolved workspace: freshly minted if this resolve was
    /// a transition, otherwise the one that workspace was announced under.
    /// Taken under the same lock as the commit, so it cannot belong to a
    /// different workspace than the one resolved — which is what a consumer
    /// sending the pair over the wire relies on.
    revision: u64,
    /// The transition to announce, if this resolve was one.
    transition: Option<Transition>,
}

impl ActiveWorkspace {
    /// Record `workspace_dir` as current.
    fn publish(&mut self, workspace_dir: &Path) -> Published {
        self.current = Some(workspace_dir.to_path_buf());
        if self.announced.as_deref() == Some(workspace_dir) {
            return Published {
                revision: self.revision,
                transition: None,
            };
        }
        self.announced = Some(workspace_dir.to_path_buf());
        self.revision += 1;
        Published {
            revision: self.revision,
            transition: Some(Transition {
                workspace_dir: workspace_dir.to_path_buf(),
                revision: self.revision,
            }),
        }
    }

    /// Whether `revision` is still the newest announced transition. A
    /// publisher that lost the race skips its emit; the winner has already
    /// said the right thing.
    fn is_current(&self, revision: u64) -> bool {
        self.revision == revision
    }

    /// Forget the resolved answer because a marker that decides it was
    /// written. Leaves `announced` alone — see the field's own note.
    fn invalidate(&mut self) -> bool {
        self.current.take().is_some()
    }
}

static ACTIVE_WORKSPACE: Lazy<RwLock<ActiveWorkspace>> =
    Lazy::new(|| RwLock::new(ActiveWorkspace::default()));

/// Record `workspace_dir` as the workspace this process is serving, and
/// return the revision it is current under.
///
/// Called after a resolution that used the real process environment.
/// Publishes [`DomainEvent::ActiveWorkspaceChanged`](crate::core::events::DomainEvent)
/// when the value actually changes, so consumers of a long-lived stream learn
/// about a switch without polling — and so the switch itself becomes a
/// visible row in the Event Log rather than an invisible reason its contents
/// changed.
///
/// The revision comes back from the same lock acquisition that committed the
/// workspace. A caller that needs the pair must take it from here rather than
/// reading the workspace and the revision separately: a switch between two
/// reads pairs workspace A with B's revision, and a receiver comparing
/// revisions then ranks a stale A above the B it should yield to.
pub(crate) fn publish_active_workspace(workspace_dir: &Path) -> u64 {
    let published = match ACTIVE_WORKSPACE.write() {
        Ok(mut guard) => guard.publish(workspace_dir),
        Err(error) => {
            log::warn!("{LOG_PREFIX} active workspace slot poisoned: {error}");
            return 0;
        }
    };

    let Some(transition) = published.transition else {
        return published.revision;
    };

    // Re-checked after the lock was dropped and before the emit: another
    // resolver may have committed a newer workspace in between, in which case
    // it has already announced the right answer and this one is stale.
    match ACTIVE_WORKSPACE.read() {
        Ok(guard) if !guard.is_current(transition.revision) => {
            log::debug!(
                "{LOG_PREFIX} dropping a superseded announcement for {} (revision {})",
                transition.workspace_dir.display(),
                transition.revision
            );
            return published.revision;
        }
        Ok(_) => {}
        Err(error) => {
            log::warn!("{LOG_PREFIX} active workspace slot poisoned: {error}");
            return published.revision;
        }
    }

    log::info!(
        "{LOG_PREFIX} active workspace is now {} (revision {})",
        transition.workspace_dir.display(),
        transition.revision
    );
    // Published with no lock held: a subscriber is free to reach back into
    // the config layer, and holding the slot across the publish would make
    // that a deadlock.
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ActiveWorkspaceChanged {
        workspace_dir: transition.workspace_dir,
        revision: transition.revision,
    });
    published.revision
}

/// Write-through hook for [`Config::load_or_init`](crate::openhuman::config::Config).
///
/// That is the process's real load path and has ~80 direct callers, so it is
/// the earliest and most frequent moment the answer is known — which is what
/// keeps the cache fresh enough for the Event Log to stamp events without
/// resolving anything itself.
///
/// Deliberately hung off `load_or_init` rather than
/// `load_or_init_with_env_lookup`: that one takes an injected `EnvLookup` so
/// tests can drive the `OPENHUMAN_WORKSPACE` branch against a fixture, and
/// publishing from there would make one test's temp directory the whole
/// binary's idea of the active workspace.
///
/// Shaped for `Result::inspect`, which is why it takes `&Config`.
pub(crate) fn publish_loaded_workspace(config: &crate::openhuman::config::Config) {
    publish_active_workspace(&config.workspace_dir);
}

/// The active workspace as last resolved, or `None` when a marker has been
/// rewritten since and nothing has resolved yet.
///
/// Synchronous and I/O-free — safe on a hot path. A caller that can afford
/// the disk read should use
/// [`active_workspace_dir`](super::dirs::active_workspace_dir) instead,
/// which is authoritative and refills this slot as a side effect.
pub fn active_workspace_dir_cached() -> Option<PathBuf> {
    match ACTIVE_WORKSPACE.read() {
        Ok(guard) => guard.current.clone(),
        Err(error) => {
            log::warn!("{LOG_PREFIX} active workspace slot poisoned: {error}");
            None
        }
    }
}

/// Drop the cached value because a marker that decides it was just written.
///
/// Clearing rather than overwriting is deliberate: a marker write says the
/// answer changed, not what it changed *to*. `active_user.toml` names a user
/// id, and turning that into a workspace is the resolver's job, including
/// the fallbacks that apply when the marker is absent or unreadable.
/// Guessing here would put a second, subtly different resolution rule in the
/// codebase — the failure mode #5334 came from.
pub(crate) fn invalidate_active_workspace() {
    match ACTIVE_WORKSPACE.write() {
        Ok(mut guard) => {
            if guard.invalidate() {
                log::debug!("{LOG_PREFIX} cleared after a workspace marker write");
            }
        }
        Err(error) => log::warn!("{LOG_PREFIX} active workspace slot poisoned: {error}"),
    }
}

#[cfg(test)]
#[path = "active_workspace_tests.rs"]
mod tests;
