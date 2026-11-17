//! Live task reservations shared by the three memory background drivers.
//!
//! The start gate makes checking peer activity and reserving a driver's own
//! slot one synchronous decision. Each driver keeps a distinct registry so a
//! migration never appears as a live import or backfill (and vice versa).

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use tinymemory_tools::run_slots::{RunGuard, RunSlots, RunStartGate, StartGate};

pub(crate) static START_GATE: LazyLock<RunStartGate> = LazyLock::new(RunStartGate::new);
pub(crate) static BACKFILL: LazyLock<RunSlots<PathBuf>> = LazyLock::new(RunSlots::new);
pub(crate) static IMPORT: LazyLock<RunSlots<PathBuf>> = LazyLock::new(RunSlots::new);
pub(crate) static MIGRATION: LazyLock<RunSlots<PathBuf>> = LazyLock::new(RunSlots::new);

/// Reserves `key` for `slots` only when no peer owns it.
///
/// The gate is held across every peer check and the reservation. The returned
/// guard belongs to this driver only and must live inside its task future.
pub(crate) fn reserve(
    slots: &RunSlots<PathBuf>,
    key: &Path,
    peers: &[&RunSlots<PathBuf>],
) -> Option<RunGuard<PathBuf>> {
    let key = key.to_path_buf();
    let mut guard = None;
    let reserved = START_GATE.with_gate(&mut || {
        if peers.iter().any(|peer| peer.is_running(&key)) {
            return false;
        }
        guard = slots.try_acquire(key.clone());
        guard.is_some()
    });
    reserved.then_some(guard).flatten()
}
