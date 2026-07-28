//! Tests for the sweep trigger's in-flight guard.
//!
//! One test function on purpose: both assertions read `SWEEP_RUNNING`, and
//! separate `#[test]`s would race each other inside the shared test process.

use super::*;

#[test]
fn the_in_flight_flag_is_never_left_set() {
    // Outside a runtime there is nothing to spawn onto, so the trigger returns
    // before claiming the flag — otherwise the first such call would disable
    // re-embedding for the rest of the process.
    ensure_vector_reembed();
    assert!(
        !SWEEP_RUNNING.load(Ordering::SeqCst),
        "a trigger that spawns nothing must not claim the flag"
    );

    // A sweep that unwinds must still release the flag. The panicking scope
    // runs on its own thread rather than behind a swapped panic hook: the hook
    // is process-global, and replacing it disturbs every test running in
    // parallel.
    SWEEP_RUNNING.store(true, Ordering::SeqCst);
    let outcome = std::thread::spawn(|| {
        let _guard = SweepGuard;
        panic!("sweep died mid-batch");
    })
    .join();

    assert!(outcome.is_err(), "the test's own panic must have unwound");
    assert!(
        !SWEEP_RUNNING.load(Ordering::SeqCst),
        "a panicking sweep must not disable re-embedding for the process"
    );
}
