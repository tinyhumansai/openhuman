//! The long-lived, context-compressed orchestrator session.
//!
//! Unlike the legacy per-tick path (`engine::run_agent`), which builds a
//! fresh `Agent`, runs it once, and discards the history, a
//! [`LongLivedSession`] keeps a single `Agent` alive across promoted
//! triggers. Its in-memory history accumulates and is compressed by the
//! `Agent`'s own context pipeline (microcompact / autocompact); the full
//! transcript is persisted to a reserved conversation thread for audit and
//! cold-boot resume.
//!
//! Concurrency: a `run_lock` serializes promoted-trigger processing so at
//! most one session run is in flight (matching the legacy single-tick
//! invariant). A monotonic `generation` counter labels each run and is the
//! hook the event loop's interrupt path (slice 5) uses to detect a
//! superseded run.
//!
//! Only *promoted* triggers reach this session — the gate has already
//! decided they're worth a reasoning-tier run.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::openhuman::agent::Agent;
use crate::openhuman::config::schema::SubconsciousMode;
use crate::openhuman::config::Config;
use crate::openhuman::memory_conversations::ConversationMessage;
use crate::openhuman::security::AutonomyLevel;

use super::engine::tick_origin_source;

/// Reserved conversation thread backing the background orchestrator's
/// internal reasoning. Distinct from the user-facing thread (slice 6).
pub const ORCHESTRATOR_THREAD_ID: &str = "subconscious:orchestrator";

/// Per-tool-call timeout injected into the session agent config (mirrors
/// the legacy tick path).
const TOOL_CALL_TIMEOUT_SECS: u64 = 5 * 60;

/// Outcome of processing one promoted trigger.
#[derive(Debug, Clone)]
pub struct ProcessOutcome {
    /// The run's monotonic generation number.
    pub generation: u64,
    /// The agent's final response text.
    pub response: String,
    pub response_chars: usize,
}

/// A persistent orchestrator session bound to one reserved thread.
pub struct LongLivedSession {
    workspace_dir: PathBuf,
    thread_id: String,
    mode: SubconsciousMode,
    /// Built lazily on first promoted trigger, then reused so history
    /// (and its compaction) persists across triggers.
    agent: Mutex<Option<Agent>>,
    /// Serializes promoted-trigger processing.
    run_lock: Mutex<()>,
    /// Monotonic run counter / supersession hook.
    generation: AtomicU64,
    /// Sticky taint: once any tainted (external-content) trigger is processed,
    /// the untrusted payload lives on in the persistent history, so every
    /// subsequent run stays `SubconsciousTainted` — otherwise a later benign
    /// cron run could regain external-effect tool access while the context
    /// still contains the earlier untrusted content.
    tainted: AtomicBool,
}

impl LongLivedSession {
    /// Create a session backed by the reserved orchestrator thread.
    pub fn new(workspace_dir: PathBuf, mode: SubconsciousMode) -> Self {
        Self::with_thread(workspace_dir, mode, ORCHESTRATOR_THREAD_ID.to_string())
    }

    /// Create a session backed by an explicit thread id (used by the
    /// user-facing thread in slice 6 and by tests).
    pub fn with_thread(workspace_dir: PathBuf, mode: SubconsciousMode, thread_id: String) -> Self {
        Self {
            workspace_dir,
            thread_id,
            mode,
            agent: Mutex::new(None),
            run_lock: Mutex::new(()),
            generation: AtomicU64::new(0),
            tainted: AtomicBool::new(false),
        }
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// The generation number of the most recently *started* run (0 before
    /// any run). Used by the event loop to detect supersession.
    pub fn latest_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Process one promoted trigger: persist it to the reserved thread,
    /// run the (persistent, compacting) agent, persist the reply.
    ///
    /// Serialized via `run_lock`. `external_content` escalates the turn
    /// origin to the tainted automation source so the approval gate
    /// refuses external-effect tools.
    pub async fn process_promoted(
        &self,
        summary: &str,
        external_content: bool,
    ) -> Result<ProcessOutcome, String> {
        let _run = self.run_lock.lock().await;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        debug!(
            "[subconscious::session] processing promoted trigger gen={} thread={} external={}",
            generation, self.thread_id, external_content
        );

        let config = Config::load_or_init()
            .await
            .map_err(|e| format!("load config: {e}"))?;

        let response = {
            let mut guard = self.agent.lock().await;
            if guard.is_none() {
                // Cold boot: build the agent and restore the persisted taint
                // marker from the reserved thread so untrusted history from a
                // previous process keeps the session tainted.
                let agent = self.build_agent(&config, summary)?;
                *guard = Some(agent);
            }

            // Sticky taint: tainted if the trigger carries external content OR
            // the session has *ever* ingested external content (still in the
            // reused/restored history).
            if external_content {
                self.tainted.store(true, Ordering::SeqCst);
            }
            let effective_tainted = self.tainted.load(Ordering::SeqCst);

            // Persist the promoted user-turn (with its taint marker) before the
            // run so the audit log + cold-boot taint restore are correct even
            // if the run fails mid-way.
            self.persist_message("user", summary, effective_tainted);

            let agent = guard.as_mut().expect("agent built above");

            let origin = crate::openhuman::agent::turn_origin::AgentTurnOrigin::TrustedAutomation {
                job_id: format!("subconscious:session:{}:{}", self.thread_id, generation),
                source: tick_origin_source(effective_tainted),
            };
            crate::openhuman::agent::turn_origin::with_origin(origin, agent.run_single(summary))
                .await
                .map_err(|e| {
                    warn!("[subconscious::session] agent run failed gen={generation}: {e}");
                    format!("agent run: {e}")
                })?
        };

        self.persist_message("agent", &response, false);

        let response_chars = response.chars().count();
        info!(
            "[subconscious::session] promoted trigger done gen={} thread={} response_chars={}",
            generation, self.thread_id, response_chars
        );
        Ok(ProcessOutcome {
            generation,
            response,
            response_chars,
        })
    }

    /// Build the session agent with mode-appropriate autonomy + iteration
    /// caps, seeding history from the reserved thread for cold-boot resume.
    fn build_agent(&self, config: &Config, current_message: &str) -> Result<Agent, String> {
        let effective = effective_config(config, self.mode);
        // Build as the `subconscious` agent (not the default orchestrator) so
        // the session's promoted turns get the subconscious tool surface —
        // memory_diff + agent_prepare_context + global to-dos/goals + the
        // notify_user user-handoff tool.
        let mut agent = Agent::from_config_for_agent(&effective, "subconscious").map_err(|e| {
            warn!("[subconscious::session] agent init failed: {e}");
            format!("agent init: {e}")
        })?;
        agent.set_event_context(self.thread_id.clone(), "subconscious");

        // Cold-boot resume: prime history from the reserved thread.
        match crate::openhuman::memory_conversations::get_messages(
            self.workspace_dir.clone(),
            &self.thread_id,
        ) {
            Ok(prior) if !prior.is_empty() => {
                // Restore the persisted taint marker: if any prior turn was
                // tainted, the untrusted content is about to be seeded back
                // into history, so the session must stay tainted.
                if prior.iter().any(|m| {
                    m.extra_metadata.get("tainted").and_then(|v| v.as_bool()) == Some(true)
                }) {
                    self.tainted.store(true, Ordering::SeqCst);
                }
                let pairs: Vec<(String, String)> =
                    prior.into_iter().map(|m| (m.sender, m.content)).collect();
                if let Err(err) = agent.seed_resume_from_messages(pairs, current_message) {
                    warn!(
                        "[subconscious::session] seed resume failed thread={} err={}",
                        self.thread_id, err
                    );
                }
            }
            Ok(_) => {
                debug!(
                    "[subconscious::session] no prior messages to seed thread={} — first run",
                    self.thread_id
                );
            }
            Err(err) => {
                warn!(
                    "[subconscious::session] reading prior messages failed thread={} err={}",
                    self.thread_id, err
                );
            }
        }
        Ok(agent)
    }

    /// Best-effort append to the reserved thread. Persistence failures are
    /// logged but never fail the run (the in-memory history is the working
    /// set; the thread is audit/resume only).
    fn persist_message(&self, sender: &str, content: &str, tainted: bool) {
        // `append_message` requires the thread to exist; the reserved
        // orchestrator thread is created lazily here (idempotent).
        ensure_reserved_thread(
            &self.workspace_dir,
            &self.thread_id,
            "Subconscious Orchestrator",
        );
        let message = new_message(sender, content, tainted);
        if let Err(err) = crate::openhuman::memory_conversations::append_message(
            self.workspace_dir.clone(),
            &self.thread_id,
            message,
        ) {
            warn!(
                "[subconscious::session] persist {} message failed thread={} err={}",
                sender, self.thread_id, err
            );
        }
    }
}

/// Apply mode-appropriate autonomy + iteration caps to a config clone.
/// Mirrors `engine::run_agent`'s effective-config logic so the legacy tick
/// and the long-lived session behave identically per mode.
pub(crate) fn effective_config(config: &Config, mode: SubconsciousMode) -> Config {
    let mut effective = config.clone();
    effective.agent.agent_timeout_secs = TOOL_CALL_TIMEOUT_SECS;
    match mode {
        SubconsciousMode::Simple => {
            effective.autonomy.level = AutonomyLevel::ReadOnly;
            effective.agent.max_tool_iterations = 15;
        }
        SubconsciousMode::Aggressive | SubconsciousMode::EventDriven => {
            effective.autonomy.level = AutonomyLevel::Full;
            effective.agent.max_tool_iterations = 30;
        }
        SubconsciousMode::Off => {}
    }
    effective
}

/// Ensure a reserved conversation thread exists before appending to it.
/// Idempotent — safe to call before every append. Failures are logged and
/// swallowed (persistence is best-effort audit, never load-bearing).
pub(crate) fn ensure_reserved_thread(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    title: &str,
) {
    use crate::openhuman::memory_conversations::CreateConversationThread;
    let req = CreateConversationThread {
        id: thread_id.to_string(),
        title: title.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        parent_thread_id: None,
        labels: None,
        personality_id: None,
    };
    if let Err(err) =
        crate::openhuman::memory_conversations::ensure_thread(workspace_dir.to_path_buf(), req)
    {
        warn!(
            "[subconscious::session] ensure reserved thread failed thread={} err={}",
            thread_id, err
        );
    }
}

/// Construct a `ConversationMessage` for the reserved thread with a fresh
/// uuid and an RFC3339 timestamp. `sender` is `"user"` or `"agent"`. The
/// `tainted` marker is persisted so a cold-boot restore can keep the session
/// tainted when untrusted history is seeded back in.
fn new_message(sender: &str, content: &str, tainted: bool) -> ConversationMessage {
    ConversationMessage {
        id: uuid::Uuid::new_v4().to_string(),
        content: content.to_string(),
        message_type: "text".to_string(),
        extra_metadata: serde_json::json!({
            "origin": "subconscious_session",
            "tainted": tainted,
        }),
        sender: sender.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_thread_id_is_stable() {
        assert_eq!(ORCHESTRATOR_THREAD_ID, "subconscious:orchestrator");
        let s = LongLivedSession::new(PathBuf::from("/tmp/ws"), SubconsciousMode::Simple);
        assert_eq!(s.thread_id(), "subconscious:orchestrator");
        assert_eq!(s.latest_generation(), 0);
    }

    #[test]
    fn with_thread_overrides_id() {
        let s = LongLivedSession::with_thread(
            PathBuf::from("/tmp/ws"),
            SubconsciousMode::Aggressive,
            "subconscious:user".to_string(),
        );
        assert_eq!(s.thread_id(), "subconscious:user");
    }

    #[test]
    fn new_message_roundtrips_sender_and_content() {
        let user = new_message("user", "hello", true);
        assert_eq!(user.sender, "user");
        assert_eq!(user.content, "hello");
        assert_eq!(user.message_type, "text");
        assert!(!user.id.is_empty());
        assert_eq!(
            user.extra_metadata.get("tainted").and_then(|v| v.as_bool()),
            Some(true)
        );
        let agent = new_message("agent", "reply", false);
        assert_eq!(agent.sender, "agent");
        assert_eq!(
            agent
                .extra_metadata
                .get("tainted")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        // Distinct ids per message.
        assert_ne!(user.id, agent.id);
    }

    #[test]
    fn effective_config_simple_is_readonly_15_iters() {
        let cfg = Config::default();
        let eff = effective_config(&cfg, SubconsciousMode::Simple);
        assert_eq!(eff.autonomy.level, AutonomyLevel::ReadOnly);
        assert_eq!(eff.agent.max_tool_iterations, 15);
        assert_eq!(eff.agent.agent_timeout_secs, TOOL_CALL_TIMEOUT_SECS);
    }

    #[test]
    fn effective_config_aggressive_is_full_30_iters() {
        let cfg = Config::default();
        let eff = effective_config(&cfg, SubconsciousMode::Aggressive);
        assert_eq!(eff.autonomy.level, AutonomyLevel::Full);
        assert_eq!(eff.agent.max_tool_iterations, 30);
    }
}
