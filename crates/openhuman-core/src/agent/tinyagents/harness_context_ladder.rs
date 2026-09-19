//! The context ladder for
//! [`assemble_turn_harness`](super::harness_assembly::assemble_turn_harness):
//! register the reduction steps that keep a turn's request inside the model's
//! context window (compression, microcompact, the final-call wrap-up, the
//! offloaded-artifact contents list and the trim) in the order they must fire.
//!
//! Split out of `harness_assembly.rs` unchanged, so the assembly stays under the
//! 750-line Rust layout limit. The call site sits at the same point in the
//! registration sequence, so every middleware keeps its position.

use std::sync::Arc;

use tinyagents_harness::middleware::ContextCompressionMiddleware;
use tinyagents_harness::runtime::AgentHarness;

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::middleware;
use crate::agent::tinyagents::model::TurnChatModel;
use crate::agent::tinyagents::summarize;
use crate::agent::tinyagents::turn_outcome::ToolOutcomeSink;

/// Push the context ladder onto `harness` and return the two handles the run
/// loop reads after the drive future returns: the installed compression
/// middleware (to drain its provenance records), and the wrap-up middleware's
/// fired flag. Each is `None` when that step is not installed.
///
/// `wrap_up_at_cap` is true only for a top-level turn that pauses at its
/// model-call cap; see the wrap-up comment below for why sub-agents and
/// erroring runs are excluded.
#[allow(clippy::too_many_arguments)]
pub(super) fn install_context_ladder(
    harness: &mut AgentHarness<(), OpenHumanRunContext>,
    model: &str,
    context_window: Option<u64>,
    autocompact_enabled: bool,
    microcompact_keep_recent: usize,
    summarizer_model: TurnChatModel,
    wrap_up_at_cap: bool,
    tool_outcome_sink: &ToolOutcomeSink,
) -> (
    Option<Arc<ContextCompressionMiddleware>>,
    Option<Arc<std::sync::atomic::AtomicBool>>,
) {
    // Autocompaction parity: when the provider's context window is known, install
    // the two-stage context-management step (issue #4249).
    //
    // 1. `ContextCompressionMiddleware` — the **summarization** step. Once the
    //    running token estimate crosses `window * SUMMARIZE_THRESHOLD_FRACTION`
    //    (90% of *this model's* context window), it folds the older slice of the
    //    transcript into a single LLM-generated system summary (keeping system
    //    messages + the recent window verbatim). This is keyed to whatever model
    //    the turn is running on, preserving the legacy context threshold.
    // 2. `ImageAwareMessageTrimMiddleware` — a deterministic, no-extra-LLM-call
    //    hard cap (issue #4462; replaces the crate `MessageTrimMiddleware`).
    //    Pushed **after** compression (so `before_model` runs compression first),
    //    it front-trims to the legacy proportional budget only as a last resort
    //    when even the summary + recent window still overflow — image markers
    //    priced flat, system messages never dropped, evictions logged.
    //
    // The LLM summarization step honors the `[context].enabled` /
    // `autocompact_enabled` opt-outs (a disabled config must not spend summarizer
    // tokens or rewrite history); the deterministic trim backstop always installs
    // when a window is known, matching the legacy always-on `trim_history` cap.
    // Concrete handle to the compression middleware (when installed), retained so
    // the run loop can drain its `records()` after the drive future returns and
    // surface each compaction's provenance (source ids + before/after token
    // estimates) — the `AgentEvent::Compressed` projection only carries the token
    // deltas, so provenance would otherwise be dropped (issue #4249, 03.1 item 6).
    let mut compression_mw: Option<Arc<ContextCompressionMiddleware>> = None;
    if let Some(window) = context_window.filter(|w| *w > 0) {
        if autocompact_enabled {
            let policy = summarize::summarization_policy(window);
            // Wrap the LLM-backed summarizer in a fault-tolerant, per-turn-caching
            // adapter (issue #4461): a summarizer failure must no longer abort the
            // turn (warn + circuit-breaker + deterministic trim instead), and an
            // identical re-issued input slice must not re-run the summarizer LLM.
            let summarizer = summarize::FaultTolerantCachingSummarizer::new(
                Box::new(summarize::ModelSummarizer::new(summarizer_model, model)),
                &policy,
            );
            let mw = Arc::new(ContextCompressionMiddleware::with_summarizer(
                policy,
                Box::new(summarizer),
            ));
            harness.push_middleware(mw.clone());
            compression_mw = Some(mw);
        }

        // Deterministic hard-cap trim (issue #4462). The crate
        // `MessageTrimMiddleware` regressed three legacy `token_budget.rs`
        // guards: it priced a base64 image at ~2M tokens (chars/4) and could
        // evict system messages, it reordered system messages to the front, and
        // its budget was the fixed `window − AGENT_TURN_MAX_OUTPUT_TOKENS`
        // (floored 1024) that collapses an 8k local model's input budget from
        // ~7373 to 1024. Our seam-owned `ImageAwareMessageTrimMiddleware`
        // restores all three: image markers priced at a flat cost, the
        // proportional reply reserve, system messages always kept in place, and a
        // grep-able warn with drop/token counts on any eviction.
    }

    // ── The context ladder, cheapest sufficient step first (issue #6014) ──────
    //
    // `before_model` runs in registration order, so what follows IS the firing
    // order, and each step only matters when the one above was not enough:
    // 1. compression (above) folds the older slice into a task-aware summary;
    // 2. microcompact blanks older tool bodies if still over;
    // 3. the wrap-up undoes (2) for a capped turn's concluding call;
    // 4. trim evicts oldest whole messages if still over.
    //
    // The order used to be 2 -> 1 -> 4, because `context_mw.install` registered
    // microcompact: the summarizer was handed a transcript whose tool bodies
    // were already `CLEARED_PLACEHOLDER` and asked, by its own prompt, for "key
    // results/outputs" it could no longer see. Nothing recovered them.
    if microcompact_keep_recent > 0 {
        // The token-budget gate the crate added for this (#4755) and this call
        // site never used. Constructed bare, microcompact blanks on EVERY call
        // past `keep_recent` — not only under pressure — so a ten-call turn
        // answered from the last five results while well inside a window with
        // room for all ten. Not a capped-turn problem: every turn. The budget
        // matches the trim's allowance, putting microcompact strictly behind
        // compression. With no window there is nothing to size against and
        // nothing else bounding growth, so always-blank stays as the backstop.
        let microcompact = tinyagents_harness::middleware::MicrocompactMiddleware::new(
            microcompact_keep_recent,
            crate::agent::context::CLEARED_PLACEHOLDER,
        );
        let microcompact = match context_window.filter(|w| *w > 0) {
            Some(window) => {
                microcompact.with_token_budget(middleware::legacy_max_input_tokens(window).max(1))
            }
            None => microcompact,
        };
        // Emit `AgentEvent::Compressed` when a body is cleared. Off by default —
        // the middleware was built as "a silent transcript rewrite" — which is
        // why blanking was, until now, the one reduction step nobody could see
        // happening: compression logs its provenance, the trim warns on every
        // eviction, and the per-result artifact store logs each persist. Only
        // this one destroyed content without saying so, which is how it went
        // unnoticed that it was doing it on every call.
        harness.push_middleware(Arc::new(microcompact.with_events(true)));
    }

    // Issue #6014: make the last permitted model call the turn's conclusion,
    // rather than leaving the answer to an extra out-of-band call after the loop
    // has exited.
    //
    // **Top-level turns only**, the same cut `CapPauser`'s dispatch guard takes.
    // A sub-agent reaching its own cap is a routine outcome, not a user-visible
    // dead end: it summarises, hands the result back to its parent, and
    // `subagent_runner` already owns that checkpoint. The harm this fixes — a
    // person left with a status line where an answer should be — belongs to the
    // turn that answers a human. It also leaves a child's budget alone: the
    // conclusion costs a call, and turning every delegated run's N tool rounds
    // into N-1 is not a trade to impose as a side effect.
    // One allowance, split between the two things that add to the request
    // (CodeRabbit on #6068). Computed independently they were each bounded and
    // the pair was not: restoration could fill the whole allowance and the
    // contents list add its tenth on top. `0` when no window is advertised.
    let trim_allowance = context_window
        .filter(|w| *w > 0)
        .map(|w| middleware::legacy_max_input_tokens(w).max(1))
        .unwrap_or(0);
    let (toc_allowance, restore_allowance) = middleware::split_input_allowance(trim_allowance);

    let wrap_up_mw = wrap_up_at_cap.then(|| {
        Arc::new(middleware::FinalCallWrapUpMiddleware::new(
            crate::agent::session_host::turn_checkpoint::MAX_ITER_CHECKPOINT_INSTRUCTION,
            tool_outcome_sink.clone(),
            // What is left after the contents list's share, so restoration
            // stops short of provoking an eviction (see the middleware).
            restore_allowance,
        ))
    });
    let wrap_up_fired = wrap_up_mw.as_ref().map(|mw| mw.fired());
    if let Some(mw) = wrap_up_mw {
        harness.push_middleware(mw);
    }

    // Issue #6014: say where the offloaded results went, from the index rather
    // than the transcript. After the reduction steps so it renders what
    // survived them, before the trim so its message is counted in that budget.
    // Its share of the allowance split above: the list is a system message, so
    // nothing downstream can shrink it (see the middleware).
    harness.push_middleware(Arc::new(middleware::ArtifactIndexTocMiddleware::new(
        toc_allowance,
    )));

    if let Some(window) = context_window.filter(|w| *w > 0) {
        // Deterministic hard-cap trim (issue #4462), last in the ladder: it drops
        // whole messages, so it runs only once summarizing and blanking have both
        // failed to fit the window. See `ImageAwareMessageTrimMiddleware` for the
        // three legacy guards it restores over the crate trim.
        harness.push_middleware(Arc::new(
            middleware::ImageAwareMessageTrimMiddleware::for_context_window(window),
        ));
    }

    (compression_mw, wrap_up_fired)
}
