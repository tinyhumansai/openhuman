//! Process-wide cancellation registry for running workflow engine loops.
//!
//! A stop/resume hand-off replaces the per-run signal. Every loop retains its
//! exact generation, so late cleanup from a fenced loop cannot erase or cancel
//! the signal now owned by its replacement.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tinyagents_harness::CancellationToken;

/// One generation of a run's legacy poll flag and crate-native token.
#[derive(Clone)]
pub(super) struct WorkflowCancelSignal {
    generation: u64,
    pub(super) flag: Arc<AtomicBool>,
    pub(super) token: CancellationToken,
}

#[derive(Default)]
struct CancelRegistry {
    next_generation: u64,
    signals: HashMap<String, WorkflowCancelSignal>,
}

fn cancel_registry() -> &'static Mutex<CancelRegistry> {
    static REGISTRY: OnceLock<Mutex<CancelRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(CancelRegistry::default()))
}

fn fresh_signal(registry: &mut CancelRegistry) -> WorkflowCancelSignal {
    registry.next_generation = registry.next_generation.wrapping_add(1);
    WorkflowCancelSignal {
        generation: registry.next_generation,
        flag: Arc::new(AtomicBool::new(false)),
        token: CancellationToken::new(),
    }
}

/// Register a signal for a new loop, preserving a stop intent that was
/// recorded before that loop was scheduled.
pub(super) fn register_cancel_signal(run_id: &str) -> WorkflowCancelSignal {
    let mut registry = cancel_registry().lock().expect("cancel registry poisoned");
    if let Some(signal) = registry.signals.get(run_id) {
        return signal.clone();
    }
    let signal = fresh_signal(&mut registry);
    registry.signals.insert(run_id.to_owned(), signal.clone());
    signal
}

/// Replace a completed or fenced loop's signal with a new generation. Resume
/// calls this only after its durable lifecycle CAS has won.
pub(super) fn replace_cancel_signal(run_id: &str) -> WorkflowCancelSignal {
    let mut registry = cancel_registry().lock().expect("cancel registry poisoned");
    let signal = fresh_signal(&mut registry);
    registry.signals.insert(run_id.to_owned(), signal.clone());
    signal
}

/// Look up the actual current signal for a run, if any.
pub(super) fn lookup_cancel_signal(run_id: &str) -> Option<WorkflowCancelSignal> {
    cancel_registry()
        .lock()
        .expect("cancel registry poisoned")
        .signals
        .get(run_id)
        .cloned()
}

/// Cancel this exact generation only while it remains current. Lifecycle
/// callers retain the signal they observed before their durable CAS, so a
/// losing stop cannot cancel a successor installed by resume.
pub(super) fn cancel_signal_if_current(run_id: &str, signal: &WorkflowCancelSignal) -> bool {
    let current = lookup_cancel_signal(run_id);
    if current
        .as_ref()
        .is_none_or(|current| current.generation != signal.generation)
    {
        return false;
    }
    signal.flag.store(true, Ordering::SeqCst);
    signal.token.cancel();
    true
}

/// Whether this loop still owns the registry slot it was created with.
pub(super) fn is_current_cancel_signal(run_id: &str, signal: &WorkflowCancelSignal) -> bool {
    lookup_cancel_signal(run_id).is_some_and(|current| current.generation == signal.generation)
}

/// Drop a loop's signal only if no stop/resume hand-off has installed a newer
/// generation in the meantime.
pub(super) fn clear_cancel_signal(run_id: &str, signal: &WorkflowCancelSignal) {
    let mut registry = cancel_registry().lock().expect("cancel registry poisoned");
    if registry
        .signals
        .get(run_id)
        .is_some_and(|current| current.generation == signal.generation)
    {
        registry.signals.remove(run_id);
    }
}

#[cfg(test)]
#[path = "cancel_tests.rs"]
mod tests;
