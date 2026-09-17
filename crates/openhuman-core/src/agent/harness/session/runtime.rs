//! Public accessors, `run_single` / `run_interactive` CLI helpers, and
//! assorted per-turn static helpers (id-fallback injection, event-error
//! sanitisation, history diffing).
//!
//! These used to live alongside the turn loop in `agent.rs`. Splitting
//! them out keeps `turn.rs` focused on the interaction lifecycle and
//! makes it obvious which methods are cheap getters vs which actually
//! drive the model.
//!
//! Each child module contributes one `impl Agent` block:
//!
//! | File            | Role                                                  |
//! |-----------------|-------------------------------------------------------|
//! | `accessors`     | Getters / setters and the tool-visibility filter.     |
//! | `resume`        | Cold-boot resume seeding of the LLM context.          |
//! | `turn_results`  | Usage totals, cap flag, and citations of the last turn.|
//! | `turn_helpers`  | Static per-turn parsing and telemetry helpers.        |
//! | `run_loop`      | `run_single` and `run_interactive`.                   |

mod accessors;
mod primary_turn;
mod resume;
mod run_loop;
mod turn_helpers;
mod turn_results;

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
