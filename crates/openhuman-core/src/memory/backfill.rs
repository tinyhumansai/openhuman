//! Storing past chats: conversations from before turns were logged.
//!
//! The lifecycle logs each turn as it happens (`lifecycle::hooks`). This walks
//! the thread store instead and stores every earlier message of every thread
//! the same way — one conversation item per message, user message `2n` and
//! its reply `2n + 1`, at the main agent's node (`agent:<id>` under the
//! layout root) — plus a [`BACKFILL_TAG`] so they can be told apart.
//!
//! The turn builder is `tinymemory_tools::backfill`; this file keeps the thread-store
//! reading, the state file, consent and the run.
//!
//! - **Turns.** A thread's messages become turns the way the chat shows them:
//!   each user message opens a turn and the replies after it, up to the next
//!   user message, are its answer. Replies before any user message form a turn
//!   with no user side.
//! - **No overlap with live logging.** A thread's turns from the first one the
//!   lifecycle logged on are already in memory; the backfill stops short of
//!   them (it asks the engine where logging began).
//! - **Resumable and repeatable.** How far each thread has been stored is kept
//!   in `<workspace>/memory/conversations_backfill.json`, so an interrupted run
//!   resumes and a later run sends only what is new. A re-sent item is a
//!   replay (its id is a content digest), never a duplicate.
//! - **Accepted, then visible.** Every batch but the last goes in with
//!   accepted writes: the engine returns once it holds them durably, and
//!   indexes them after. The last waits until it is visible, and the scope's
//!   log is ordered, so a `done` run is searchable. An engine without
//!   accepted writes stores every batch as before.
//! - **Consent.** It uploads chat history to the selected engine, so
//!   [`start`] refuses without `consent`.

use crate::core::runtime::DispatchCtx;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::memory::engine::{self, BoundEngine};
use crate::memory::error::{MemoryError, MemoryResult};
use crate::memory::ops::store_many_on_with;
use crate::memory::scope::{MemoryIdentity, ResolvedIdentity};
use crate::memory::types::ImportPhase;
use crate::threads::store::blocking as threads;

use tinymemory_tools::backfill::{first_live_turn, group_items, turn_groups, ThreadMessage};
pub use tinymemory_tools::backfill::{pending_range, turns_of, PastTurn, BACKFILL_TAG, MAIN_AGENT};

/// Backfills running now, per workspace.
static RUNNING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Progress of the last (or current) backfill.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillState {
    /// Idle, running, done, or stopped on an error.
    pub phase: ImportPhase,
    /// Threads with turns to store when the run started.
    pub threads_total: u64,
    /// Of those, threads finished.
    pub threads_done: u64,
    /// Turns stored by the run.
    pub turns_stored: u64,
    /// Conversation items stored by the run.
    pub items_stored: u64,
    /// Why it stopped, when it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// When the last run finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

/// `memory_conversations_backfill_status` / `_start` result.
#[derive(Debug, Clone, Serialize)]
pub struct BackfillView {
    /// The run's progress.
    pub state: BackfillState,
    /// Threads that still have earlier turns to store.
    pub pending_threads: u64,
    /// Turns still to store across them.
    pub pending_turns: u64,
}

/// `memory_conversations_backfill_start` params.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BackfillStartParams {
    /// The caller agrees to upload past chats to the engine.
    #[serde(default)]
    pub consent: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BackfillFile {
    #[serde(default)]
    state: BackfillState,
    /// Turns stored so far, per thread.
    #[serde(default)]
    stored: BTreeMap<String, u32>,
}

const STATE: &str = "conversations_backfill";

fn read_file(workspace_dir: &Path) -> BackfillFile {
    super::state_store::load(workspace_dir, STATE)
        .unwrap_or_else(|error| {
            tracing::warn!(error = %error, "[memory:backfill] reading state failed");
            None
        })
        .unwrap_or_default()
}

/// Persists `file` atomically (one storage write), so a crash mid-write keeps
/// the previous progress instead of losing it.
fn write_file(workspace_dir: &Path, file: &BackfillFile) {
    if let Err(error) = super::state_store::save(workspace_dir, STATE, file) {
        tracing::warn!(error = %error, "[memory:backfill] writing state failed");
    }
}

/// One thread's plan: its turns and the range still to store.
struct ThreadPlan {
    thread_id: String,
    turns: Vec<PastTurn>,
    range: std::ops::Range<usize>,
}

async fn plan(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    bound: Option<&BoundEngine>,
    stored: &BTreeMap<String, u32>,
) -> MemoryResult<Vec<ThreadPlan>> {
    let listed = threads::list_threads(ctx.clone(), workspace_dir.to_path_buf())
        .await
        .map_err(|error| MemoryError::Engine(format!("reading chat threads failed: {error}")))?;
    let mut plans = Vec::new();
    for thread in listed {
        let messages = match threads::get_messages(
            ctx.clone(),
            workspace_dir.to_path_buf(),
            thread.id.clone(),
        )
        .await
        {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(error = %error, "[memory:backfill] a thread was unreadable; skipped");
                continue;
            }
        };
        let messages: Vec<ThreadMessage> = messages
            .into_iter()
            .map(|m| ThreadMessage::new(m.sender, m.content, m.created_at))
            .collect();
        let turns = turns_of(&messages);
        let first_live = match bound {
            Some(bound) => first_live_turn(bound.engine.as_ref(), &thread.id).await,
            None => None,
        };
        let range = pending_range(
            turns.len(),
            first_live,
            stored.get(&thread.id).copied().unwrap_or(0),
        );
        if !range.is_empty() {
            plans.push(ThreadPlan {
                thread_id: thread.id,
                turns,
                range,
            });
        }
    }
    Ok(plans)
}

fn is_running(workspace_dir: &Path) -> bool {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(workspace_dir)
}

/// `memory_conversations_backfill_status`.
pub async fn status(ctx: &DispatchCtx, config: &Config) -> MemoryResult<BackfillView> {
    let workspace_dir = &config.workspace_dir;
    let file = read_file(workspace_dir);
    let mut state = file.state;
    if state.phase == ImportPhase::Running && !is_running(workspace_dir) {
        state.phase = ImportPhase::Error;
        state.error = Some("the sync was interrupted; start it again to resume".to_string());
    }
    let bound = engine::resolve(config).engine().ok();
    let plans = plan(ctx, workspace_dir, bound.as_ref(), &file.stored).await?;
    Ok(BackfillView {
        state,
        pending_threads: plans.len() as u64,
        pending_turns: plans.iter().map(|p| p.range.len() as u64).sum(),
    })
}

/// `memory_conversations_backfill_start`: requires `consent` and memory on.
pub async fn start(
    ctx: &DispatchCtx,
    config: &Config,
    params: BackfillStartParams,
) -> MemoryResult<BackfillView> {
    if !params.consent {
        return Err(MemoryError::invalid(
            "storing past chats uploads them to the selected engine; pass consent: true",
        ));
    }
    let bound = engine::resolve(config).engine()?;
    let workspace_dir = config.workspace_dir.clone();
    let claimed = RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(workspace_dir.clone());
    if !claimed {
        return status(ctx, config).await;
    }
    let mut file = read_file(&workspace_dir);
    let plans = match plan(ctx, &workspace_dir, Some(&bound), &file.stored).await {
        Ok(plans) => plans,
        Err(error) => {
            release(&workspace_dir);
            return Err(error);
        }
    };
    let pending_turns: u64 = plans.iter().map(|p| p.range.len() as u64).sum();
    file.state = BackfillState {
        phase: ImportPhase::Running,
        threads_total: plans.len() as u64,
        ..BackfillState::default()
    };
    write_file(&workspace_dir, &file);
    let view = BackfillView {
        state: file.state.clone(),
        pending_threads: plans.len() as u64,
        pending_turns,
    };
    tracing::info!(
        threads = plans.len(),
        turns = pending_turns,
        "[memory:backfill] started"
    );
    let identity = MemoryIdentity::agent(MAIN_AGENT).resolve(config);
    tokio::spawn(async move {
        run(&workspace_dir, &bound, &identity, file, plans).await;
        release(&workspace_dir);
    });
    Ok(view)
}

fn release(workspace_dir: &Path) {
    RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(workspace_dir);
}

async fn run(
    workspace_dir: &Path,
    bound: &BoundEngine,
    identity: &ResolvedIdentity,
    mut file: BackfillFile,
    plans: Vec<ThreadPlan>,
) {
    let last = last_batch(identity, &plans);
    for (thread, plan) in plans.into_iter().enumerate() {
        // Groups of whole turns, so the stored counter never splits a turn.
        for (index, group) in turn_groups(plan.range.clone()).into_iter().enumerate() {
            let items = group_items(
                &identity.layout,
                &identity.agent_id,
                &plan.thread_id,
                &plan.turns,
                &group,
            );
            let count = items.len() as u64;
            if count > 0 {
                // Accepted, not indexed: waiting on the engine to index every
                // batch made a few thousand messages take hours on CortexDB.
                // The last batch waits until it is visible: the scope's log
                // is ordered, so everything stored before it is then too, and
                // `done` still means searchable.
                let options = if last == Some((thread, index)) {
                    tinymemory_api::WriteOptions::visible()
                } else {
                    tinymemory_api::WriteOptions::accepted()
                };
                let stored = store_many_on_with(bound, items, options).await;
                if let Err(error) = stored {
                    tracing::warn!(
                        code = error.code(),
                        "[memory:backfill] storing a batch failed; stopping"
                    );
                    file.state.phase = ImportPhase::Error;
                    file.state.error = Some(String::from(error));
                    file.state.finished_at = Some(Utc::now());
                    write_file(workspace_dir, &file);
                    return;
                }
            }
            let end = group.last().map_or(0, |last| last + 1);
            file.stored.insert(
                plan.thread_id.clone(),
                u32::try_from(end).unwrap_or(u32::MAX),
            );
            file.state.turns_stored += group.len() as u64;
            file.state.items_stored += count;
            write_file(workspace_dir, &file);
        }
        file.state.threads_done += 1;
        write_file(workspace_dir, &file);
    }
    file.state.phase = ImportPhase::Done;
    file.state.finished_at = Some(Utc::now());
    write_file(workspace_dir, &file);
    tracing::info!(
        items = file.state.items_stored,
        turns = file.state.turns_stored,
        "[memory:backfill] done"
    );
}

/// The last batch of the run that stores anything, as (thread, group): the
/// one the run waits on until it is visible.
fn last_batch(identity: &ResolvedIdentity, plans: &[ThreadPlan]) -> Option<(usize, usize)> {
    plans.iter().enumerate().rev().find_map(|(thread, plan)| {
        turn_groups(plan.range.clone())
            .iter()
            .enumerate()
            .rev()
            .find(|(_, group)| {
                !group_items(
                    &identity.layout,
                    &identity.agent_id,
                    &plan.thread_id,
                    &plan.turns,
                    group,
                )
                .is_empty()
            })
            .map(|(index, _)| (thread, index))
    })
}

#[cfg(test)]
#[path = "backfill_tests.rs"]
mod tests;
