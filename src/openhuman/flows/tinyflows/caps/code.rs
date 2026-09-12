//! The `CodeRunner` capability for `code` nodes.
//!
//! Runs JavaScript or Python inside the sandbox, under a wall-clock timeout, so
//! a runaway or hostile snippet cannot hold a flow run open indefinitely.

#![allow(unused_imports)]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::*;
use tinyflows::error::{EngineError, Result};

use super::*;
use crate::openhuman::agent::harness::definition::SandboxMode;
use crate::openhuman::config::Config;
use crate::openhuman::sandbox::{execute_in_sandbox, resolve_sandbox_policy};
use crate::openhuman::security::{CommandClass, SecurityPolicy};

/// [`CodeRunner`] adapter running sandboxed user code via
/// `src/openhuman/sandbox/ops.rs` (`resolve_sandbox_policy` +
/// `execute_in_sandbox`), modeled on
/// `src/openhuman/tools/impl/system/node_exec.rs::run_sandboxed`.
///
/// **Mismatch handled here:** the sandbox runs a shell command string, not a
/// `(language, source, input)` triple. `source` is treated as a function body
/// receiving the serialized `input` items array and returning the node's
/// output — this convention is a B1 design choice (not specified by the
/// crate), matching the mock's "function body" tests
/// (`tinyflows::nodes::integration::code` — e.g. `"source": "return 1;"`).
///
/// Requires `node`/`python3` on the `PATH` the sandbox backend runs under;
/// there is no managed toolchain wiring here (unlike `node_exec`'s
/// `NodeBootstrap`).
///
/// **Phase 2 — autonomy-tier gating:** a `code` node runs arbitrary user code
/// in a sandbox, so it is treated as [`CommandClass::Write`] (state-changing but
/// sandbox-bounded — not inherently catastrophic). Before dispatch it consults
/// [`enforce_node_tier_gate`]: a read-only run `Block`s and never executes; a
/// Supervised run then routes through the `ApprovalGate` (Write ⇒ `Prompt`); a
/// Full run executes silently. This closes the prior gap where the code node had
/// no policy check and no approval gate at all.
pub struct OpenHumanCode {
    pub config: Arc<Config>,
    pub security: Arc<SecurityPolicy>,
}

pub(crate) const CODE_RUN_TIMEOUT_SECS: u64 = 60;

#[async_trait]
impl CodeRunner for OpenHumanCode {
    async fn run(&self, language: CodeLanguage, source: &str, input: Value) -> Result<Value> {
        // Autonomy-tier gate (Phase 2): sandboxed arbitrary-code execution is
        // Write-class. A read-only run `Block`s here and never spawns anything;
        // Supervised/Full fall through to the ApprovalGate below.
        let tier_decision = enforce_node_tier_gate(&self.security, CommandClass::Write, "code")?;

        // Approval gate (mirrors OpenHumanTools/OpenHumanHttp): `gate_call_for_tier`
        // is what turns a Supervised-tier `Prompt` decision into a real human
        // round-trip before any code runs — escalating past the flow's own
        // `require_approval` toggle when the tier itself says "ask me" (Codex P1).
        // A Deny short-circuits. The audit summary is computed on a redacted view
        // of the request, never the raw source secrets, matching the other
        // acting adapters.
        let action = json!({ "language": format!("{language:?}"), "source": source });
        let summary = crate::openhuman::security::approval::summarize_action("flows_code", &action);
        let redacted = crate::openhuman::security::approval::redact_args(&action);
        let (gate_outcome, audit_id) =
            gate_call_for_tier(tier_decision, "flows_code", &summary, redacted).await;
        if let crate::openhuman::security::approval::GateOutcome::Deny { reason } = gate_outcome {
            return Err(EngineError::Capability(reason));
        }

        let outcome: Result<Value> = async {
        let policy = resolve_sandbox_policy(
            SandboxMode::Sandboxed,
            &self.config.action_dir,
            &self.config.runtime,
            false,
        );

        // Work dir lives under `action_dir` (the sandbox workspace root). We keep
        // its path *relative* to `action_dir` so the run command works on every
        // backend: for Local, `execute_in_sandbox`'s `working_dir` is the host
        // cwd; for Docker, `action_dir` is bind-mounted at `/workspace` with
        // `-w /workspace`. Host-absolute paths would not exist inside the
        // container, so we pass `action_dir` as the working dir and reference the
        // script/input by their `action_dir`-relative paths.
        let rel_dir = std::path::Path::new(".flows_code").join(uuid::Uuid::new_v4().to_string());
        let work_dir = self.config.action_dir.join(&rel_dir);
        tokio::fs::create_dir_all(&work_dir)
            .await
            .map_err(|e| EngineError::Capability(format!("failed to create code work dir: {e}")))?;

        // Keep every fallible staging and dispatch step inside one result so
        // the cleanup below runs for serialization/write failures as well as
        // sandbox failures.
        let exec_result = async {
            let (script_name, interpreter, script_body) = match language {
                CodeLanguage::JavaScript => ("script.js", "node", js_harness(source)),
                CodeLanguage::Python => ("script.py", "python3", python_harness(source)),
            };
            let script_path = work_dir.join(script_name);
            let input_path = work_dir.join("input.json");

            let input_json = serde_json::to_string(&input).map_err(|e| {
                EngineError::Capability(format!("failed to serialize code input: {e}"))
            })?;
            tokio::fs::write(&script_path, script_body)
                .await
                .map_err(|e| EngineError::Capability(format!("failed to write code script: {e}")))?;
            tokio::fs::write(&input_path, input_json)
                .await
                .map_err(|e| EngineError::Capability(format!("failed to write code input: {e}")))?;

            // Backend-agnostic, `action_dir`-relative command paths (see above).
            let rel_script = rel_dir.join(script_name);
            let rel_input = rel_dir.join("input.json");
            let command = format!(
                "{} {} {}",
                shell_quote(interpreter),
                shell_quote(&rel_script.to_string_lossy()),
                shell_quote(&rel_input.to_string_lossy()),
            );

            let mut extra_env = std::collections::HashMap::new();
            if let Ok(host_path) = std::env::var("PATH") {
                extra_env.insert("PATH".into(), host_path.into());
            }

            tracing::debug!(
                target: "flows",
                ?language,
                work_dir = %work_dir.display(),
                "[flows] code: running sandboxed script"
            );

            execute_in_sandbox(
                &policy,
                &command,
                &self.config.action_dir,
                extra_env,
                std::time::Duration::from_secs(CODE_RUN_TIMEOUT_SECS),
            )
            .await
            .map_err(|e| EngineError::Capability(format!("sandbox execution failed: {e}")))
        }
        .await;

        // Always clean up the work dir — even when `execute_in_sandbox` itself
        // errors (e.g. a spawn failure) — so temp scripts never leak.
        if let Err(e) = tokio::fs::remove_dir_all(&work_dir).await {
            tracing::debug!(target: "flows", error = %e, "[flows] code: failed to clean up work dir (non-fatal)");
        }

        let result = exec_result?;

        if !result.success() {
            return Err(EngineError::Capability(format!(
                "code node exited non-zero (timed_out={}): {}",
                result.timed_out, result.stderr
            )));
        }

        serde_json::from_str(result.stdout.trim())
            .map_err(|e| EngineError::Capability(format!("code output was not valid JSON: {e}")))
        }
        .await;

        // Close out the approval audit with the run's success/failure (mirrors
        // OpenHumanTools/OpenHumanHttp).
        if let Some(id) = audit_id {
            if let Some(gate) = crate::openhuman::security::approval::ApprovalGate::try_global() {
                let exec = if outcome.is_ok() {
                    crate::openhuman::security::approval::ExecutionOutcome::Success
                } else {
                    crate::openhuman::security::approval::ExecutionOutcome::Failure
                };
                gate.record_execution(
                    &id,
                    exec,
                    outcome.as_ref().err().map(ToString::to_string).as_deref(),
                );
            }
        }

        outcome
    }
}

/// Wraps user `source` as a function body receiving `input`, executed by Node,
/// printing the JSON result (or `null`) to stdout.
pub(crate) fn js_harness(source: &str) -> String {
    format!(
        "const fs = require('fs');\n\
         const input = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));\n\
         const __result__ = (function(input) {{\n{source}\n}})(input);\n\
         process.stdout.write(JSON.stringify(__result__ === undefined ? null : __result__));\n"
    )
}

/// Wraps user `source` as a function body receiving `input`, executed by
/// Python, printing the JSON result (or `null`) to stdout.
pub(crate) fn python_harness(source: &str) -> String {
    let indented: String = if source.trim().is_empty() {
        "    pass".to_string()
    } else {
        source
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "import sys, json\n\
         with open(sys.argv[1]) as __f__:\n    input = json.load(__f__)\n\
         def __user_fn__(input):\n{indented}\n    return None\n\
         __result__ = __user_fn__(input)\n\
         print(json.dumps(__result__))\n"
    )
}

/// POSIX single-quote shell escaping, mirroring
/// `tools/impl/system/node_exec.rs::shell_quote`.
pub(crate) fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(test)]
#[path = "code_tests.rs"]
mod tests;
