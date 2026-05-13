//! Scheduler gate — gates background AI work on host conditions.
//!
//! Background AI tasks (memory-tree digests, embeddings, summarisation) used
//! to run flat-out and made the host visibly lag, especially on battery.
//! This module exposes a single decision point — [`current_policy`] — that
//! background workers consult before spending CPU/GPU on LLM-bound work.
//!
//! Signals (refreshed every 30s in a background sampler):
//!   * power state — on AC, or battery >= 80%
//!   * CPU usage — recent global usage; <70% means "idle enough"
//!   * deployment mode — server/container hosts run flat-out
//!
//! Resulting [`Policy`]:
//!   * [`Policy::Aggressive`] — server-mode; bypass throttles entirely
//!   * [`Policy::Normal`] — desktop with headroom; run as scheduled
//!   * [`Policy::Throttled`] — busy or on battery; serialise + slow down
//!   * [`Policy::Paused`] — user opted out; defer indefinitely
//!
//! Cooperative throttling: callers `await gate::wait_for_capacity()` before
//! each unit of LLM-bound work. The future resolves immediately in
//! Aggressive/Normal, sleeps in Throttled, and re-polls in Paused so the
//! caller resumes the moment the user toggles the gate back on.

pub mod gate;
pub mod policy;
pub mod signals;

pub use gate::{
    current_policy, current_signals, init_global, is_signed_out, set_signed_out, wait_for_capacity,
    LlmPermit,
};
pub use policy::{PauseReason, Policy};
pub use signals::Signals;

#[cfg(test)]
pub(crate) use gate::SignedOutTestGuard;
