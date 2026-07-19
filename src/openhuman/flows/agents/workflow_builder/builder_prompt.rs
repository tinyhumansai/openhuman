//! Server-side turn-prompt construction for the `workflow_builder` agent.
//!
//! This is the Rust home of what used to live in the frontend
//! (`app/src/lib/flows/workflowBuilderPrompt.ts`): the natural-language brief
//! that kicks off a builder turn. Moving it here makes the builder a
//! first-class backend agent — `flows::ops::flows_build` runs the agent
//! directly (like the Flow Scout), instead of the frontend crafting delegate
//! strings and relying on the chat orchestrator to route them.
//!
//! Persistence contract: every mode is PROPOSE-ONLY — saving always stays
//! behind the user's explicit action (the copilot panel's Accept, then the
//! canvas's own Save). [`BuildMode::Build`] is the instant-create path (the
//! host already made the blank flow), so its brief injects that flow id as
//! future-turn context but explicitly forbids `save_workflow` on this turn:
//! rejecting the proposal must leave the flow's persisted graph untouched
//! (see issue #4596). Enabling/disabling a flow is never in scope here.

use serde::Deserialize;
use serde_json::Value;

/// Which authoring turn to run. Selects the leading directive + how the current
/// graph / context is injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildMode {
    /// First draft from a free-text description; returns a proposal only.
    Create,
    /// Iterative refine of the injected draft; returns the revised proposal.
    Revise,
    /// Diagnose a failed run and propose a corrected graph.
    Repair,
    /// Instant-create: the flow already exists (blank), so build → dry-run →
    /// propose against `flow_id`. Persistence still waits on the copilot
    /// panel's Accept + the canvas's Save; the agent must NOT `save_workflow`
    /// here.
    Build,
}

/// A structured builder-turn request. Replaces the four ad-hoc prompt builders
/// the frontend used to assemble; the handler passes one of these and the
/// server renders the brief.
#[derive(Debug, Clone, Deserialize)]
pub struct BuilderRequest {
    /// Which kind of turn to run.
    pub mode: BuildMode,
    /// The user's ask: the description (`create`/`build`) or the change
    /// instruction (`revise`), or a short note (`repair`, optional).
    #[serde(default)]
    pub instruction: String,
    /// The current draft graph, injected as context for `revise`/`repair`/`build`.
    #[serde(default)]
    pub graph: Option<Value>,
    /// The saved flow's id (required for `build`; optional elsewhere so the
    /// agent may `run_flow` it to test after confirming).
    #[serde(default)]
    pub flow_id: Option<String>,
    /// The failed run id (== thread id) for `repair`, so the agent can
    /// `get_flow_run` it.
    #[serde(default)]
    pub run_id: Option<String>,
    /// The run-level error message for `repair`, if known.
    #[serde(default)]
    pub error: Option<String>,
    /// Node ids implicated in the failure, for `repair`, if known.
    #[serde(default)]
    pub failing_node_ids: Vec<String>,
}

impl BuilderRequest {
    /// Validates a builder-turn request before prompt rendering.
    ///
    /// [`BuildMode::Build`] injects a `flow_id` as context for future turns
    /// (the user may later ask the agent to save/test that flow). A missing or
    /// blank `flow_id` would render `The flow's id is ``.` into the brief and
    /// contradict the "instant-create flow already exists" framing, so reject
    /// it here (the RPC path deserializes `BuilderRequest` directly, where
    /// only `mode` is required).
    pub fn validate(&self) -> Result<(), String> {
        if self.mode == BuildMode::Build
            && self
                .flow_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        {
            return Err("flows_build: `flow_id` is required for build mode".to_string());
        }
        Ok(())
    }
}

/// A leading directive that frames the turn's persistence contract.
const DIRECTIVE_PROPOSE: &str =
    "Design a tinyflows automation and return a workflow proposal for me to review. \
     Do not save, enable, or run anything.";

const DIRECTIVE_REVISE: &str = "Revise this tinyflows automation and return the revised proposal. Do not save \
     unless I explicitly ask you to (when I do, use save_workflow on the saved flow id), and never enable or \
     disable anything. If I ask you to run/test the SAVED flow, follow the run_flow capability rule from \
     your standing instructions: only run_flow it if that tool is on your belt and only after you confirm \
     with me first; if it isn't on your belt, point me to the Run control in the Workflows UI instead of \
     offering.";

const DIRECTIVE_BUILD_PROPOSE_ONLY: &str = "Build this tinyflows automation END-TO-END and return the workflow \
     proposal. The flow already exists (created blank just now) — design the graph and verify it with \
     dry_run_workflow, then return the proposal for me to review. Do NOT save_workflow in this turn — \
     I will review the proposal in the copilot panel, accept it onto the canvas draft, and save it \
     myself. Do not enable, disable, or run_flow anything unless I explicitly confirm first.";

/// Serialize a graph compactly for injection as agent context.
fn serialize_graph(graph: &Value) -> String {
    serde_json::to_string(graph).unwrap_or_else(|_| "{}".to_string())
}

/// Renders the natural-language brief for a builder turn from a structured
/// request. This is the single server-side source of the builder's turn text.
#[must_use]
pub fn render_prompt(req: &BuilderRequest) -> String {
    let instruction = req.instruction.trim();
    match req.mode {
        BuildMode::Create => {
            format!("{DIRECTIVE_PROPOSE}\n\nBuild a workflow that does this:\n{instruction}")
        }
        BuildMode::Revise => {
            let mut lines = vec![
                DIRECTIVE_REVISE.to_string(),
                String::new(),
                "Here is the current workflow draft (tinyflows WorkflowGraph JSON):".to_string(),
                "```json".to_string(),
                req.graph
                    .as_ref()
                    .map(serialize_graph)
                    .unwrap_or_else(|| "{}".to_string()),
                "```".to_string(),
            ];
            if let Some(flow_id) = req.flow_id.as_deref().filter(|s| !s.is_empty()) {
                lines.push(String::new());
                lines.push(format!(
                    "This workflow is saved with flow id `{flow_id}` — if I ask you to run/test it, follow \
                     the run_flow capability rule: only run_flow that id if the tool is on your belt and \
                     I've confirmed first; otherwise point me to the Run control in the Workflows UI."
                ));
            }
            lines.push(String::new());
            lines.push("Revise it as follows and return the full revised proposal:".to_string());
            lines.push(instruction.to_string());
            lines.join("\n")
        }
        BuildMode::Build => {
            let flow_id = req.flow_id.as_deref().unwrap_or("");
            [
                DIRECTIVE_BUILD_PROPOSE_ONLY,
                "",
                &format!(
                    "The flow's id is `{flow_id}` (kept for future turns — do not save_workflow it here). \
                     Its current (blank) graph is:"
                ),
                "```json",
                &req.graph
                    .as_ref()
                    .map(serialize_graph)
                    .unwrap_or_else(|| "{}".to_string()),
                "```",
                "",
                "Build a workflow that does this:",
                instruction,
            ]
            .join("\n")
        }
        BuildMode::Repair => {
            let run_id = req.run_id.as_deref().unwrap_or("(unknown)");
            let mut parts = vec![
                DIRECTIVE_PROPOSE.to_string(),
                String::new(),
                format!(
                    "A run of this workflow failed (run id: {run_id}). Read the run with get_flow_run, \
                     diagnose why it failed, and propose a fix."
                ),
            ];
            if let Some(err) = req
                .error
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                parts.push(String::new());
                parts.push(format!("Run error: {err}"));
            }
            if !req.failing_node_ids.is_empty() {
                parts.push(String::new());
                parts.push(format!(
                    "Failing step node id(s): {}",
                    req.failing_node_ids.join(", ")
                ));
            }
            if let Some(graph) = req.graph.as_ref() {
                parts.push(String::new());
                parts.push(
                    "Here is the current workflow draft (tinyflows WorkflowGraph JSON):"
                        .to_string(),
                );
                parts.push("```json".to_string());
                parts.push(serialize_graph(graph));
                parts.push("```".to_string());
            }
            if !instruction.is_empty() {
                parts.push(String::new());
                parts.push(instruction.to_string());
            }
            parts.push(String::new());
            parts.push("Return the full corrected proposal.".to_string());
            parts.join("\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(mode: BuildMode) -> BuilderRequest {
        BuilderRequest {
            mode,
            instruction: "email me a digest every morning".to_string(),
            graph: None,
            flow_id: None,
            run_id: None,
            error: None,
            failing_node_ids: vec![],
        }
    }

    #[test]
    fn create_prompt_frames_propose_only() {
        let p = render_prompt(&req(BuildMode::Create));
        assert!(p.contains("Do not save, enable, or run"));
        assert!(p.contains("email me a digest every morning"));
    }

    #[test]
    fn revise_injects_graph_and_flow_id() {
        let mut r = req(BuildMode::Revise);
        r.instruction = "add a Slack step".into();
        r.graph = Some(json!({ "nodes": [], "edges": [] }));
        r.flow_id = Some("flow_42".into());
        let p = render_prompt(&r);
        assert!(p.contains("```json"));
        assert!(p.contains("flow_42"));
        assert!(p.contains("add a Slack step"));
    }

    #[test]
    fn revise_run_guidance_is_capability_conditional() {
        // Regression: the revise-turn directive (and its per-turn flow_id
        // note) used to unconditionally assert "you may run_flow" —
        // contradicting the standing prompt's capability check (Bld §4),
        // which hides run_flow/resume_flow_run/cancel_flow_run on the
        // flows_build path (`FLOWS_BUILD_HIDDEN_TOOLS`). Because the
        // per-turn brief is appended AFTER the standing prompt, an
        // unconditional per-turn assertion would override the standing
        // prompt's capability check and reproduce the offer-then-refuse bug
        // the standing-prompt fix was meant to close. Both the mode-level
        // directive and the flow_id-specific note must defer to the
        // capability rule instead of asserting the tool is available.
        let mut r = req(BuildMode::Revise);
        r.flow_id = Some("flow_77".into());
        let p = render_prompt(&r);

        assert!(
            p.contains("run_flow capability rule"),
            "revise directive must defer to the run_flow capability rule rather than \
             assert the tool is available"
        );
        assert!(
            p.contains("Run control in the Workflows UI"),
            "revise directive must point to the Workflows UI Run control as the \
             off-the-belt fallback"
        );

        for banned in [
            "You may run_flow the SAVED flow to test it, but ONLY if I ask",
            "may run_flow that id, but confirm with me first.",
        ] {
            assert!(
                !p.contains(banned),
                "revise directive must not carry the stale unconditional run_flow \
                 phrasing `{banned}`"
            );
        }
    }

    #[test]
    fn build_is_propose_only_and_injects_flow_id_as_context() {
        // Regression for #4596: the instant-create build turn must NOT
        // instruct the agent to `save_workflow`. Rejecting the proposal has
        // to leave the created-blank flow's persisted graph untouched, so
        // persistence stays behind the copilot panel's Accept + the canvas's
        // Save. The flow id is still injected as context for future turns.
        let mut r = req(BuildMode::Build);
        r.flow_id = Some("flow_9".into());
        r.graph = Some(json!({ "nodes": [], "edges": [] }));
        let p = render_prompt(&r);
        // Positive: the new directive explicitly forbids save_workflow on
        // this turn.
        assert!(
            p.contains("Do NOT save_workflow"),
            "build directive must forbid save_workflow explicitly (#4596)"
        );
        // Negative: none of the old imperative-save phrasings survive
        // (any of them would put us back in the auto-save bug).
        for banned in [
            "then SAVE",
            "with save_workflow",
            "SAVE it onto",
            "save_workflow onto",
        ] {
            assert!(
                !p.contains(banned),
                "build directive must not carry auto-save phrasing `{banned}` (#4596)"
            );
        }
        // Negative (B27): the old phantom "review card" phrasing must not
        // survive — the agent echoed this verbatim to users, contradicting
        // its own auto-save behavior.
        for banned in ["review card", "Accept the proposal explicitly"] {
            assert!(
                !p.contains(banned),
                "build directive must not carry phantom review-card phrasing `{banned}` (B27)"
            );
        }
        // Context is still injected so the user can later ask the agent to
        // save/test that specific flow.
        assert!(p.contains("flow_9"));
        assert!(p.contains("END-TO-END"));
    }

    /// The standing archetype (`prompt.md`, the always-loaded system prompt —
    /// as opposed to the per-turn directives rendered above) carries the same
    /// B27 banned-phrase regression, plus positive coverage for the plain-
    /// language style rule and the read-only memory grounding tool added
    /// alongside it. Guards against reintroducing jargon-leaking or
    /// phantom-review-card language, and against silently losing the
    /// `memory_recall` guidance if the prompt is ever rewritten.
    #[test]
    fn standing_prompt_teaches_plain_language_and_readonly_memory() {
        const STANDING_PROMPT: &str = include_str!("prompt.md");

        // Negative (B27): the phantom "review card" phrasing must never
        // reappear in the standing prompt either.
        for banned in ["review card", "Accept the proposal explicitly"] {
            assert!(
                !STANDING_PROMPT.contains(banned),
                "standing prompt must not carry phantom review-card phrasing `{banned}` (B27)"
            );
        }

        // Positive: the anti-jargon Style rule — replies must stay in plain
        // language, never leak response_format/schema/expression internals.
        assert!(
            STANDING_PROMPT.contains("Speak to a non-technical user"),
            "standing prompt must teach the anti-jargon Style rule"
        );

        // Positive: read-only memory grounding via the raw `memory_recall`
        // tool (no `memory_store` — see the agent.toml regression test).
        assert!(
            STANDING_PROMPT.contains("memory_recall"),
            "standing prompt must teach the builder to ground itself with memory_recall"
        );

        // Positive: the prompt must state the read-only contract explicitly —
        // not just mention the tool name — so a future edit can't silently
        // drop the "can't change their memory" guarantee this agent's tool
        // scope depends on (no `memory_store` in agent.toml).
        assert!(
            STANDING_PROMPT.contains("Read-only — you can't change their memory"),
            "standing prompt must state the memory read-only guarantee, not just mention memory_recall"
        );

        // Negative (contract accuracy, issue #6): `create_workflow` and
        // `duplicate_flow` are on this agent's belt (see agent.toml's `named`
        // tool list), so the prompt must never claim the agent can't create a
        // flow at all — only that it can't enable/run one unattended.
        for banned in [
            "create a new flow, or enable/disable one",
            "It cannot create flows,",
        ] {
            assert!(
                !STANDING_PROMPT.contains(banned),
                "standing prompt must not carry the stale \"can never create a flow\" claim \
                 `{banned}` — create_workflow/duplicate_flow are on the belt (issue #6)"
            );
        }

        // Positive: the accurate contract — the agent CAN create a flow, but
        // every flow it creates is always born disabled.
        assert!(
            STANDING_PROMPT.contains("create_workflow") && STANDING_PROMPT.contains("born"),
            "standing prompt must accurately teach that create_workflow exists and that \
             created flows are always born disabled (issue #6)"
        );

        // Positive (Bld §4): run guidance is capability-conditional. `run_flow`
        // (and resume/cancel) are hidden on the `flows_build` path, so the
        // prompt must NOT unconditionally claim the builder can run a flow —
        // it must first check whether the tool is on its belt and, when it is
        // not, point the user to the Workflows UI Run control instead of
        // offering-then-refusing (the confusing "want me to run it?" → "I
        // don't have access" behavior).
        assert!(
            STANDING_PROMPT.contains("only if the tool is on your belt")
                && STANDING_PROMPT.contains("never offer to run the flow")
                && STANDING_PROMPT.contains("Workflows UI"),
            "standing prompt must make run_flow capability-conditional: never offer to run \
             when the tool is off the belt, and point the user to the Workflows UI Run \
             control instead (Bld §4 offer-then-refuse)"
        );

        // Negative: the pre-fix heading ("ask first!") asserted run_flow was
        // simply a confirm-before-use tool, with no capability check at all —
        // it must not reappear (that's the exact offer-then-refuse regression
        // Bld §4 closed).
        assert!(
            !STANDING_PROMPT.contains("`run_flow` (ask first!)"),
            "standing prompt must not regress to the pre-Bld-§4 unconditional \
             \"ask first!\" run_flow heading"
        );

        // Positive: the run_flow section must explicitly gate the real-run
        // instructions behind the capability check, not just mention the
        // check somewhere else in the doc — bind the assertion to the two
        // halves of the actual contract (off-belt fallback, on-belt usage).
        assert!(
            STANDING_PROMPT
                .contains("If you do **not** have a `run_flow` tool, never offer to run the flow"),
            "standing prompt must state the off-belt fallback as a direct consequence \
             of the capability check, not a generic nearby mention"
        );
        assert!(
            STANDING_PROMPT
                .contains("If you **do** have `run_flow`: once the user has **saved** a flow"),
            "standing prompt must gate the on-belt run_flow usage behind the same \
             capability check"
        );

        // Positive (CodeRabbit follow-up on Bld §4): `resume_flow_run` /
        // `cancel_flow_run` get the identical capability-conditional
        // treatment as `run_flow` — both are hidden alongside it on the
        // `flows_build` path (`FLOWS_BUILD_HIDDEN_TOOLS`), so a fix that only
        // gated `run_flow` while leaving these two unconditional would
        // reopen the same offer-then-refuse bug one hop later.
        assert!(
            STANDING_PROMPT
                .contains("those tools are on your belt** — `resume_flow_run` (approval-gated) or"),
            "standing prompt must gate resume_flow_run/cancel_flow_run behind the \
             same on-your-belt capability check as run_flow"
        );
        assert!(
            STANDING_PROMPT.contains("(if they're not available, point the"),
            "standing prompt must state the resume/cancel off-belt fallback condition"
        );
        assert!(
            STANDING_PROMPT
                .contains("user to the runs list in the Workflows UI instead of offering)."),
            "standing prompt must point resume/cancel's off-belt fallback to the \
             Workflows UI runs list, matching run_flow's UI fallback pattern"
        );

        // Negative: the pre-fix wording offered resume/cancel unconditionally
        // right after `edit_workflow`, with no capability check in between —
        // must not reappear.
        assert!(
            !STANDING_PROMPT.contains("patch with `edit_workflow`; `resume_flow_run`"),
            "standing prompt must not regress to the pre-fix unconditional \
             resume_flow_run/cancel_flow_run offer"
        );

        // Positive: self-DM resolution — the prompt must teach the builder to
        // wire "DM me" onto the connection's own `platform_user_id`, not a
        // public channel (the #general/#team-product fallback bug).
        assert!(
            STANDING_PROMPT.contains("platform_user_id"),
            "standing prompt must teach that list_flow_connections surfaces \
             platform_user_id for self-DM resolution"
        );
        assert!(
            STANDING_PROMPT.contains("DM me"),
            "standing prompt must keep the \"DM me\" self-target guidance"
        );
        assert!(
            STANDING_PROMPT.contains("Never default a personal request to a public channel"),
            "standing prompt must explicitly forbid falling back to a public \
             channel (e.g. #general/#team-product) for a personal \"DM me\" request"
        );

        // Positive: assert the *complete* wiring instruction, not just the
        // presence of the `platform_user_id` keyword — a regression could
        // drop the actual "pass it as `channel`" directive while leaving the
        // word `platform_user_id` elsewhere in the prompt and still pass the
        // looser check above.
        assert!(
            STANDING_PROMPT
                .contains("that id verbatim as the `channel` arg on `SLACK_SEND_MESSAGE`"),
            "standing prompt must explicitly instruct passing `platform_user_id` \
             verbatim as the `channel` arg on `SLACK_SEND_MESSAGE` — not just \
             mention the field name"
        );

        // Positive: the null-`platform_user_id` fallback (ask the user for
        // their member id in one question) must survive too — this is the
        // other half of the self-DM contract and must not be silently lost.
        assert!(
            STANDING_PROMPT.contains("Only if `platform_user_id` is null")
                && STANDING_PROMPT.contains("ask the user for their member id"),
            "standing prompt must preserve the null-`platform_user_id` fallback: \
             ask the user for their member id in one question rather than \
             guessing a channel"
        );

        // Positive: non-owner DM resolution — the prompt must teach the
        // builder to resolve a NAMED recipient who is NOT the connected
        // owner via a lookup node, not just the owner's own
        // `platform_user_id`. This guidance must be PLATFORM-AGNOSTIC (no
        // toolkit-specific slug hardcoded) — the same shape applies to
        // Slack, Discord, Telegram, or any other messaging toolkit.
        assert!(
            STANDING_PROMPT.contains("is NOT the connected"),
            "standing prompt must teach the non-owner DM case explicitly"
        );
        assert!(
            STANDING_PROMPT.contains("platform-agnostic"),
            "standing prompt must state the non-owner DM guidance is \
             platform-agnostic, not tied to one toolkit"
        );
        assert!(
            STANDING_PROMPT.contains("search_tool_catalog { query, toolkit }"),
            "standing prompt must teach resolving the lookup action via \
             search_tool_catalog scoped to the TARGET toolkit, rather than \
             hardcoding one platform's slug"
        );
        assert!(
            STANDING_PROMPT.contains("tool_call` node upstream of the send"),
            "standing prompt must teach wiring the lookup as a tool_call \
             node upstream of the send node"
        );
        assert!(
            STANDING_PROMPT.contains("resolves to exactly one match"),
            "standing prompt must require a name search to resolve to \
             exactly one match before binding it without asking"
        );
        assert!(
            STANDING_PROMPT.contains("ask the user to confirm which person"),
            "standing prompt must preserve the safety rule: never message an \
             unverified same-name match, ask instead when ambiguous"
        );
        assert!(
            STANDING_PROMPT.contains("Check the send action")
                && STANDING_PROMPT.contains("open conversation"),
            "standing prompt must teach checking the send tool's own contract \
             for a required open-conversation step, handled generally via the \
             contract rather than a single-platform special case"
        );

        // Negative: none of the non-owner DM guidance may hardcode a
        // toolkit-specific action slug or arg name — the reviewer flagged an
        // earlier draft of this guidance as Slack-only, which violates the
        // platform-agnostic rule.
        for banned in [
            "SLACK_FIND_USERS",
            "SLACK_LIST_ALL_USERS",
            "config.args.email",
            "exact_match",
        ] {
            assert!(
                !STANDING_PROMPT.contains(banned),
                "standing prompt's non-owner DM guidance must not hardcode \
                 the platform-specific `{banned}` — it must stay \
                 platform-agnostic (any messaging toolkit)"
            );
        }
    }

    /// The standing prompt must teach reply hygiene: no deliberation
    /// narration, no draft-then-restate, lead with substance. Without these
    /// the reasoning-tier model narrates its chain of thought in the visible
    /// reply ("let me think… actually wait… let me reconsider") and restates
    /// its questions twice in the same message. (The harness already keeps
    /// real reasoning blocks out of the visible text — this is the model
    /// choosing to narrate in its output, so a prompt rule is the fix.)
    #[test]
    fn standing_prompt_teaches_reply_hygiene() {
        const STANDING_PROMPT: &str = include_str!("prompt.md");

        for rule in [
            "finished reply",
            "No deliberation narration",
            "No draft-then-restate",
            "Lead with substance",
        ] {
            assert!(
                STANDING_PROMPT.contains(rule),
                "standing prompt must teach the reply-hygiene rule `{rule}` — the \
                 reply is the finished answer, not a thinking scratchpad (no \
                 deliberation narration, no draft-then-restate)"
            );
        }
    }

    /// Before asking the user for a missing value, the builder must exhaust
    /// self-resolution — recall, connections, tool catalog, and (for
    /// runtime-only facts like the user's own platform handle) wiring a
    /// lookup node — and only ask for genuine preferences, not resolvable
    /// facts. This also guards that the existing "zero questions is still
    /// the happy path" balance line survives: the rule must not turn into
    /// "ask about everything".
    #[test]
    fn standing_prompt_teaches_resolution_first_self_resolution() {
        const STANDING_PROMPT: &str = include_str!("prompt.md");

        for rule in [
            "asking is the last resort",
            "Wire a runtime lookup",
            "resolvable facts",
            "genuine preferences",
            "get authenticated user",
            "what you already tried",
            "zero questions is still the happy path",
        ] {
            assert!(
                STANDING_PROMPT.contains(rule),
                "standing prompt must teach the resolution-first rule `{rule}` — \
                 before asking for any missing value, the builder must exhaust \
                 self-resolution (recall, connections, tool catalog, runtime \
                 lookup) and only ask for genuine preferences, while the \
                 zero-questions happy path still holds"
            );
        }
    }

    #[test]
    fn repair_includes_run_id_error_and_failing_nodes() {
        let mut r = req(BuildMode::Repair);
        r.run_id = Some("run_7".into());
        r.error = Some("tool_call node: missing `slug`".into());
        r.failing_node_ids = vec!["send".into(), "notify".into()];
        r.graph = Some(json!({ "nodes": [], "edges": [] }));
        let p = render_prompt(&r);
        assert!(p.contains("run_7"));
        assert!(p.contains("get_flow_run"));
        assert!(p.contains("missing `slug`"));
        assert!(p.contains("send, notify"));
    }

    #[test]
    fn build_mode_deserializes_from_snake_case() {
        let r: BuilderRequest =
            serde_json::from_value(json!({ "mode": "build", "instruction": "x", "flow_id": "f1" }))
                .expect("deserialize");
        assert_eq!(r.mode, BuildMode::Build);
        assert_eq!(r.flow_id.as_deref(), Some("f1"));
    }

    #[test]
    fn validate_rejects_build_without_flow_id() {
        // Missing entirely.
        let missing = req(BuildMode::Build);
        assert!(missing.validate().is_err());

        // Present but blank / whitespace-only.
        let mut blank = req(BuildMode::Build);
        blank.flow_id = Some("   ".into());
        assert!(blank.validate().is_err());

        // A real id passes.
        let mut ok = req(BuildMode::Build);
        ok.flow_id = Some("flow_9".into());
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn validate_allows_non_build_modes_without_flow_id() {
        // Only `build` requires a flow id; the propose/revise/repair turns may run
        // without one.
        for mode in [BuildMode::Create, BuildMode::Revise, BuildMode::Repair] {
            assert!(
                req(mode).validate().is_ok(),
                "{mode:?} should not require flow_id"
            );
        }
    }
}
