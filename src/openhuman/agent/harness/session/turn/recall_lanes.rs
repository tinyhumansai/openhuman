//! The per-message memory lanes of the turn's context.
//!
//! Both lanes fetch what memory holds for *this* message and prepend it to the
//! user message — never to the system prompt, whose rendered bytes are the
//! KV-cache prefix the inference backend has already tokenised. Both are
//! bounded, because on a cold launch the memory module may still be
//! downloading behind them: a turn without a block is an ordinary turn, a turn
//! that waits minutes for one is the outage the bounds exist to prevent.
//!
//! - **Lane B** — situational preferences: topic-scoped preferences whose
//!   vector similarity to the message clears a floor. Runs every turn; an
//!   unrelated message clears the gate to nothing.
//! - **Lane C** — auto-recall of facts about the user (#6040): opens only for
//!   a message that asks about the user, then one bounded tree lookup. See
//!   [`crate::openhuman::memory::auto_recall`] for the gate, the floor and the
//!   switch.
//!
//! The two run **side by side**. Each embeds the message on its own — there is
//! no retrieval that takes a precomputed vector — so run in sequence the turn
//! would pay the sum of two round trips; joined, it pays the slower one. The
//! blocks are appended in a fixed order afterwards, so the prompt is stable
//! whichever lane answers first.
//!
//! `core_turn.rs` documents the broad per-turn recall that used to live here
//! and why it was removed; neither lane is that.

use super::super::types::Agent;
use std::time::Instant;

/// Append the Lane B and Lane C blocks for `user_message` to `context`.
pub(super) async fn append_recall_lanes(agent: &Agent, user_message: &str, context: &mut String) {
    let (situational, auto_recall) = tokio::join!(
        situational_preferences(agent, user_message),
        auto_recall_block(agent, user_message),
    );
    if !situational.is_empty() {
        context.push_str("## Relevant preferences for this message\n\n");
        for pref in &situational {
            context.push_str("- ");
            context.push_str(pref.trim());
            context.push('\n');
        }
        context.push('\n');
    }
    if let Some(block) = auto_recall {
        context.push_str(&block);
    }
}

/// Lane B: preferences semantically relevant to this message.
async fn situational_preferences(agent: &Agent, user_message: &str) -> Vec<String> {
    // 5 s, not 3: the module's lookup (a query embed round trip, queued behind
    // the citation and autosave calls spawned off the turn) measured over 3 s
    // on a live desktop and lost the block on both on-topic turns of the
    // #6041 field test.
    const SITUATIONAL_RECALL_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);
    let started = Instant::now();
    let situational = match tokio::time::timeout(
        SITUATIONAL_RECALL_BUDGET,
        crate::openhuman::memory::preferences::recall_situational_preferences_on(
            &agent.memory,
            user_message,
        ),
    )
    .await
    {
        Ok(situational) => situational,
        Err(_elapsed) => {
            log::warn!(
                "[pref_recall] situational recall exceeded {SITUATIONAL_RECALL_BUDGET:?}; \
                 continuing without a preference block"
            );
            return Vec::new();
        }
    };
    if situational.is_empty() {
        log::debug!(
            "[pref_recall] no situational preference relevant to this message elapsed_ms={}",
            started.elapsed().as_millis()
        );
    } else {
        log::info!(
            "[pref_recall] situational block injected: {} item(s) elapsed_ms={}",
            situational.len(),
            started.elapsed().as_millis()
        );
    }
    situational
}

/// Lane C: the gated auto-recall block, when the session has the lane bound.
async fn auto_recall_block(agent: &Agent, user_message: &str) -> Option<String> {
    let Some(auto_recall) = agent.auto_recall.as_ref() else {
        log::debug!("[auto_recall] no lane bound to this session; skipping");
        return None;
    };
    auto_recall.block_for(user_message).await
}
