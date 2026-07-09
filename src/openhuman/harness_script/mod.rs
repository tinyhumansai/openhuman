//! LLM-authored Python orchestration scripts over OpenHuman's harness
//! primitives (issue #4250).
//!
//! **Phase 1 (this module): runner skeleton only.** It launches a managed
//! Python subprocess via [`crate::openhuman::runtime_python`], speaks a
//! versioned newline-JSON protocol, and runs a one-shot script to a terminal
//! result/error under wall-time and output-byte caps with cooperative
//! cancellation.
//!
//! Deferred to later phases: the orchestration bridge (`agent.run`,
//! `workflow.run`, checkpoints — Phase 2), the `run_harness_script` tool
//! surface (Phase 3), static AST validation and sandboxing (Phase 4), and
//! graph observability/telemetry (Phase 5).
//!
//! No orchestration handlers are wired, so a Phase 1 script has no
//! *orchestration* side effects — it cannot spawn agents or workflows. The
//! `exec` itself is **unsandboxed** (`# PHASE 4`), so raw Python capability
//! (filesystem, network, subprocess, inherited environment) is unrestricted
//! until the Phase 4 sandbox lands; do not feed this untrusted input. The
//! mitigating factor today is that the runner is not reachable from any tool or
//! RPC surface yet, so nothing invokes it with attacker-controlled scripts.
//!
//! Design: `docs/superpowers/specs/2026-07-09-harness-script-runner-design.md`.

pub mod protocol;
pub mod runner;

pub use protocol::{ScriptLimits, PROTOCOL_VERSION};
pub use runner::{HarnessScriptError, PythonHarnessScriptRunner, ScriptRunOutcome, ScriptRunSpec};

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
