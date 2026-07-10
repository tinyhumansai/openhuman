//! Interactive plan-review gate (Codex/Claude plan-mode contract).
//!
//! When an interactive (`WebChat`) turn proposes a thread-scoped plan, the
//! orchestrator calls the [`tool::RequestPlanReviewTool`] (`request_plan_review`)
//! which parks the live turn on [`gate::PlanReviewGate`] until the user decides.
//! The UI's plan-review card resolves it via the `openhuman.plan_review_decide`
//! RPC (see [`schemas`]). Approve resumes-and-executes, Reject resumes-and-stops,
//! Revise resumes-with-feedback so the agent re-plans and re-parks.
//!
//! This is the live-turn counterpart to the task-board approval lifecycle (which
//! the background dispatcher runs on the `user-tasks` / `task-sources` boards):
//! the dispatcher never sweeps conversation thread boards, so a chat plan must be
//! gated on the turn itself, not via a board status. Modelled on
//! [`crate::openhuman::approval`] but in-memory only.

pub mod gate;
pub mod schemas;
pub mod tool;
pub mod types;

pub use schemas::all_controller_schemas as all_plan_review_controller_schemas;
pub use schemas::all_registered_controllers as all_plan_review_registered_controllers;
pub use tool::RequestPlanReviewTool;
pub use types::PlanReviewResolution;
