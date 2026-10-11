//! Storing past chats: conversations from before turns were logged.
//!
//! The lifecycle logs each turn as it happens (`lifecycle::hooks`). This walks
//! the thread store instead and stores every earlier message of every thread
//! the same way — one conversation item per message, user message `2n` and
//! its reply `2n + 1`, at the main agent's node (`agent:<id>` under the
//! layout root) — plus a [`BACKFILL_TAG`] so they can be told apart.
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
//! - **Consent.** It uploads chat history to the selected engine, so
//!   [`start`] refuses without `consent`.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tinymemory_api::{
    ItemKind, ListRequest, MemoryMeta, MetaFilter, Role, SourceKind, SourceRef, StoreItem, Turn,
    TurnRange,
};

use crate::config::Config;
use crate::memory::engine::{self, BoundEngine};
use crate::memory::error::{MemoryError, MemoryResult};
use crate::memory::ops::store_many_on;
use crate::memory::scope::{MemoryIdentity, ResolvedIdentity};
use crate::memory::types::ImportPhase;
use crate::threads::store::blocking as threads;
use crate::threads::store::ConversationMessage;

/// The tag every backfilled conversation item carries.
pub const BACKFILL_TAG: &str = "backfill";

/// The agent definition past chats belong to: the main chat agent.
pub const MAIN_AGENT: &str = "orchestrator";

/// Conversation items per bulk store (`MemoryEngine::store_many`).
const STORE_GROUP: usize = 50;

/// Items a thread lookup lists to find where live logging began.
const LIVE_SCAN_LIMIT: usize = 200;

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

fn file_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir
        .join("memory")
        .join("conversations_backfill.json")
}

fn read_file(workspace_dir: &Path) -> BackfillFile {
    std::fs::read_to_string(file_path(workspace_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Persists `file` atomically (staged, synced, renamed), so a crash
/// mid-write keeps the previous progress instead of losing it.
fn write_file(workspace_dir: &Path, file: &BackfillFile) {
    let result = serde_json::to_vec_pretty(file)
        .map_err(|error| error.to_string())
        .and_then(|json| {
            crate::security::secret_storage::file_store::write_atomic(
                &file_path(workspace_dir),
                &json,
            )
            .map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        tracing::warn!(error = %error, "[memory:backfill] writing state failed");
    }
}

fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// One past turn: what the user said and what came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastTurn {
    /// The user's message; empty for replies before any user message.
    pub user: String,
    /// The replies, joined.
    pub assistant: String,
    /// When the turn last changed.
    pub at: DateTime<Utc>,
}

/// A thread's messages as turns, oldest first.
#[must_use]
pub fn turns_of(messages: &[ConversationMessage]) -> Vec<PastTurn> {
    let mut turns: Vec<PastTurn> = Vec::new();
    for message in messages {
        let text = message.content.trim();
        if text.is_empty() {
            continue;
        }
        let at = parse_time(&message.created_at);
        if message.sender.eq_ignore_ascii_case("user") {
            turns.push(PastTurn {
                user: text.to_string(),
                assistant: String::new(),
                at: at.unwrap_or_else(Utc::now),
            });
            continue;
        }
        if turns.is_empty() {
            turns.push(PastTurn {
                user: String::new(),
                assistant: String::new(),
                at: at.unwrap_or_else(Utc::now),
            });
        }
        if let Some(turn) = turns.last_mut() {
            if !turn.assistant.is_empty() {
                turn.assistant.push_str("\n\n");
            }
            turn.assistant.push_str(text);
            if let Some(at) = at {
                turn.at = at;
            }
        }
    }
    turns
}

/// Which turns of a thread with `total` turns the backfill should store:
/// from what it already stored up to (not including) the first turn live
/// logging took, if any.
#[must_use]
pub fn pending_range(
    total: usize,
    first_live: Option<usize>,
    already_stored: u32,
) -> std::ops::Range<usize> {
    let end = first_live.map_or(total, |first| first.min(total));
    let start = (already_stored as usize).min(end);
    start..end
}

/// The conversation items for turn `index` of `thread_id`, shaped as the
/// lifecycle logs them (one per message), tagged [`BACKFILL_TAG`].
#[must_use]
pub fn turn_items(
    identity: &ResolvedIdentity,
    thread_id: &str,
    index: usize,
    turn: &PastTurn,
) -> Vec<StoreItem> {
    let Ok(node) = identity.layout.conversations(&identity.agent_id) else {
        return Vec::new();
    };
    let base = u32::try_from(index.saturating_mul(2)).unwrap_or(u32::MAX - 1);
    [
        (Role::User, &turn.user, base),
        (Role::Assistant, &turn.assistant, base + 1),
    ]
    .into_iter()
    .filter(|(_, text, _)| !text.trim().is_empty())
    .map(|(role, text, at_index)| StoreItem::Conversation {
        meta: MemoryMeta {
            namespace: node.clone(),
            thread_id: Some(thread_id.to_string()),
            turns: Some(TurnRange {
                first: at_index,
                last: at_index,
            }),
            agent_id: Some(identity.agent_id.clone()),
            source: SourceRef {
                kind: SourceKind::Conversation,
                id: Some(thread_id.to_string()),
            },
            observed_at: Some(turn.at),
            tags: vec![BACKFILL_TAG.to_string()],
            ..MemoryMeta::default()
        },
        turns: vec![Turn {
            at: Some(turn.at),
            ..Turn::new(role, text.as_str())
        }],
    })
    .collect()
}

/// The first turn of `thread_id` the lifecycle logged (not a backfill), as a
/// turn number; `None` when none was.
async fn first_live_turn(bound: &BoundEngine, thread_id: &str) -> Option<usize> {
    let page = bound
        .engine
        .list(ListRequest {
            filter: MetaFilter {
                thread_id: Some(thread_id.to_string()),
                ..MetaFilter::kinds([ItemKind::Conversation])
            },
            limit: LIVE_SCAN_LIMIT,
            cursor: None,
        })
        .await
        .map_err(|error| {
            tracing::debug!(%error, "[memory:backfill] live turn lookup failed");
        })
        .ok()?;
    page.items
        .iter()
        .filter(|hit| !hit.meta.tags.iter().any(|tag| tag == BACKFILL_TAG))
        .filter_map(|hit| hit.meta.turns.map(|turns| turns.first as usize / 2))
        .min()
}

/// One thread's plan: its turns and the range still to store.
struct ThreadPlan {
    thread_id: String,
    turns: Vec<PastTurn>,
    range: std::ops::Range<usize>,
}

async fn plan(
    workspace_dir: &Path,
    bound: Option<&BoundEngine>,
    stored: &BTreeMap<String, u32>,
) -> MemoryResult<Vec<ThreadPlan>> {
    let listed = threads::list_threads(workspace_dir.to_path_buf())
        .await
        .map_err(|error| MemoryError::Engine(format!("reading chat threads failed: {error}")))?;
    let mut plans = Vec::new();
    for thread in listed {
        let messages = match threads::get_messages(workspace_dir.to_path_buf(), thread.id.clone())
            .await
        {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(error = %error, "[memory:backfill] a thread was unreadable; skipped");
                continue;
            }
        };
        let turns = turns_of(&messages);
        let first_live = match bound {
            Some(bound) => first_live_turn(bound, &thread.id).await,
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
pub async fn status(config: &Config) -> MemoryResult<BackfillView> {
    let workspace_dir = &config.workspace_dir;
    let file = read_file(workspace_dir);
    let mut state = file.state;
    if state.phase == ImportPhase::Running && !is_running(workspace_dir) {
        state.phase = ImportPhase::Error;
        state.error = Some("the sync was interrupted; start it again to resume".to_string());
    }
    let bound = engine::resolve(config).engine().ok();
    let plans = plan(workspace_dir, bound.as_ref(), &file.stored).await?;
    Ok(BackfillView {
        state,
        pending_threads: plans.len() as u64,
        pending_turns: plans.iter().map(|p| p.range.len() as u64).sum(),
    })
}

/// `memory_conversations_backfill_start`: requires `consent` and memory on.
pub async fn start(config: &Config, params: BackfillStartParams) -> MemoryResult<BackfillView> {
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
        return status(config).await;
    }
    let mut file = read_file(&workspace_dir);
    let plans = match plan(&workspace_dir, Some(&bound), &file.stored).await {
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
    for plan in plans {
        let indices: Vec<usize> = plan.range.clone().collect();
        // Groups of whole turns, so the stored counter never splits a turn.
        for group in indices.chunks(STORE_GROUP / 2) {
            let items: Vec<StoreItem> = group
                .iter()
                .flat_map(|&index| turn_items(identity, &plan.thread_id, index, &plan.turns[index]))
                .collect();
            let count = items.len() as u64;
            if count > 0 {
                if let Err(error) = store_many_on(bound, items).await {
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

#[cfg(test)]
#[path = "backfill_tests.rs"]
mod tests;
