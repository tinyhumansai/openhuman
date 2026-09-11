//! The hook engine — matching, ordering, and aggregating.
//!
//! One process-global engine holds the merged [`HookConfig`], the session-scoped
//! environment contributed by `sessionStart` hooks, and the per-session
//! follow-up counters that stop a `stop` hook from looping forever.
//!
//! ## Gating versus observing
//!
//! [`HookEvent::is_gating`] splits the events in two, and the split decides the
//! execution strategy:
//!
//! * **Gating** events run their hooks **sequentially, in layer order**, and the
//!   turn waits. A denial short-circuits the rest — once the action is refused
//!   there is nothing later hooks can add, and running them would pay latency
//!   for an outcome that cannot change.
//! * **Observing** events are dispatched onto a background task and the turn
//!   never waits. An audit hook that hangs must not hang the agent.
//!
//! That asymmetry is the whole reason the engine exists rather than each call
//! site spawning processes itself: it is the one place the latency budget of a
//! turn is spent deliberately.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use super::config::{self, HookConfig, HookDefinition};
use super::exec::{self, HookRun, DEFAULT_TIMEOUT};
use super::matcher;
use super::types::{HookEvent, HookInput, HookOutput};

/// Follow-ups a single hook may inject per session before the engine stops
/// honouring it. Matches Cursor's default; a definition may raise it, and an
/// explicit `null` in the file lifts the cap entirely.
pub const DEFAULT_LOOP_LIMIT: u32 = 5;

/// The result of dispatching one event.
#[derive(Debug, Clone, Default)]
pub struct HookOutcome {
    /// The merged decision across every hook that ran.
    pub output: HookOutput,
    /// Per-hook detail, in run order. Empty for a fire-and-forget dispatch.
    pub runs: Vec<HookRun>,
}

impl HookOutcome {
    /// Whether the action was refused.
    pub fn is_deny(&self) -> bool {
        self.output.is_deny()
    }

    /// Whether the action needs human approval.
    pub fn is_ask(&self) -> bool {
        self.output.is_ask()
    }

    /// The reason to hand the model, when the action was refused.
    pub fn denial_reason(&self) -> Option<&str> {
        if !self.is_deny() {
            return None;
        }
        self.output
            .agent_message
            .as_deref()
            .or(self.output.user_message.as_deref())
    }
}

/// Matching, execution, and aggregation of configured hooks.
pub struct HookEngine {
    config: RwLock<Arc<HookConfig>>,
    /// Environment contributed by `sessionStart` hooks, keyed by session id.
    /// Cursor scopes these to the session; so do we, rather than mutating the
    /// process environment, which would leak one session's variables into
    /// every concurrent one.
    session_env: RwLock<HashMap<String, BTreeMap<String, String>>>,
    /// Follow-ups already granted, keyed by session and hook label.
    loop_counts: RwLock<HashMap<(String, String), u32>>,
    default_timeout: RwLock<Duration>,
}

impl Default for HookEngine {
    fn default() -> Self {
        Self {
            config: RwLock::new(Arc::new(HookConfig::default())),
            session_env: RwLock::new(HashMap::new()),
            loop_counts: RwLock::new(HashMap::new()),
            default_timeout: RwLock::new(DEFAULT_TIMEOUT),
        }
    }
}

static ENGINE: std::sync::LazyLock<HookEngine> = std::sync::LazyLock::new(HookEngine::default);

/// The process-global engine.
pub fn global() -> &'static HookEngine {
    &ENGINE
}

impl HookEngine {
    /// Re-read every layer and swap the config in.
    ///
    /// Reloading is a whole-config swap rather than an incremental patch: a
    /// half-applied hook set is a policy nobody wrote.
    pub async fn reload(
        &self,
        project_dir: Option<PathBuf>,
        workspace_dir: Option<PathBuf>,
    ) -> Arc<HookConfig> {
        let loaded = tokio::task::spawn_blocking(move || {
            config::load(project_dir.as_deref(), workspace_dir.as_deref())
        })
        .await
        .unwrap_or_default();
        for warning in &loaded.warnings {
            log::warn!("[hooks] {warning}");
        }
        let loaded = Arc::new(loaded);
        *self.config.write().await = Arc::clone(&loaded);
        log::info!(
            "[hooks] active configuration: {} hook(s) across {} file(s)",
            loaded.len(),
            loaded.sources.len()
        );
        loaded
    }

    /// Install a config directly. Used by tests and by the RPC surface, which
    /// validates a candidate file before letting it become live.
    pub async fn install(&self, config: HookConfig) {
        *self.config.write().await = Arc::new(config);
    }

    /// The currently active config.
    pub async fn snapshot(&self) -> Arc<HookConfig> {
        Arc::clone(&*self.config.read().await)
    }

    /// Override the timeout applied to hooks that name none.
    pub async fn set_default_timeout(&self, timeout: Duration) {
        *self.default_timeout.write().await = timeout;
    }

    /// Whether any hook is registered for an event. Call sites use this to skip
    /// building an input envelope for an event nobody is listening to — the
    /// common case, and the reason hooks cost nothing when unconfigured.
    pub async fn has_hooks(&self, event: HookEvent) -> bool {
        !self.snapshot().await.for_event(event).is_empty()
    }

    /// Run an event's hooks and return the merged decision.
    ///
    /// Observing events are dispatched in the background and return an empty
    /// outcome immediately; see the module docs.
    pub async fn dispatch(&self, event: HookEvent, input: HookInput) -> HookOutcome {
        let config = self.snapshot().await;
        let selected: Vec<HookDefinition> = config
            .for_event(event)
            .iter()
            .filter(|definition| definition.enabled)
            .filter(|definition| {
                matcher::matches(
                    definition.matcher.as_deref(),
                    matcher::subject(event, &input.payload),
                )
            })
            .cloned()
            .collect();
        if selected.is_empty() {
            return HookOutcome::default();
        }
        log::debug!(
            "[hooks] {event}: {} hook(s) selected ({} configured)",
            selected.len(),
            config.for_event(event).len()
        );

        if !event.is_gating() {
            self.dispatch_detached(event, input, selected).await;
            return HookOutcome::default();
        }
        self.dispatch_blocking(event, input, selected).await
    }

    /// Run an event's hooks in the foreground regardless of whether the event
    /// is gating, and report every run.
    ///
    /// Only the `hooks.test` RPC uses this. A detached dispatch would report
    /// nothing, which is precisely the opposite of what an author debugging a
    /// hook needs — so the test path trades the latency guarantee for
    /// observability, and nothing on a turn's path may call it.
    pub async fn dispatch_for_test(&self, event: HookEvent, input: HookInput) -> HookOutcome {
        let config = self.snapshot().await;
        let selected: Vec<HookDefinition> = config
            .for_event(event)
            .iter()
            .filter(|definition| definition.enabled)
            .filter(|definition| {
                matcher::matches(
                    definition.matcher.as_deref(),
                    matcher::subject(event, &input.payload),
                )
            })
            .cloned()
            .collect();
        if selected.is_empty() {
            return HookOutcome::default();
        }
        self.dispatch_blocking(event, input, selected).await
    }

    /// Run hooks in order, stopping at the first denial.
    async fn dispatch_blocking(
        &self,
        event: HookEvent,
        input: HookInput,
        selected: Vec<HookDefinition>,
    ) -> HookOutcome {
        let env = self.env_for(&input).await;
        let default_timeout = *self.default_timeout.read().await;
        let mut outcome = HookOutcome::default();
        for definition in selected {
            let run = exec::run(&definition, &input, &env, default_timeout).await;
            log::debug!(
                "[hooks] {event}: {} finished in {}ms (deny={}, error={:?})",
                run.label,
                run.duration.as_millis(),
                run.output.is_deny(),
                run.error
            );
            let denied = run.output.is_deny();
            let followup = run.output.followup_message.clone();
            let mut run = run;
            if followup.is_some() && !self.grant_followup(&input, &definition, &run.label).await {
                log::debug!(
                    "[hooks] {event}: {} exhausted its follow-up budget; dropping the message",
                    run.label
                );
                run.output.followup_message = None;
            }
            outcome.output.merge(run.output.clone());
            outcome.runs.push(run);
            if denied {
                break;
            }
        }
        if let Some(env) = outcome.output.env.clone() {
            self.absorb_session_env(&input, env).await;
        }
        outcome
    }

    /// Fire hooks on a background task, so the turn never waits on them.
    async fn dispatch_detached(
        &self,
        event: HookEvent,
        input: HookInput,
        selected: Vec<HookDefinition>,
    ) {
        let env = self.env_for(&input).await;
        let default_timeout = *self.default_timeout.read().await;
        tokio::spawn(async move {
            for definition in selected {
                let run = exec::run(&definition, &input, &env, default_timeout).await;
                if let Some(error) = &run.error {
                    log::warn!("[hooks] {event}: {} failed: {error}", run.label);
                } else {
                    log::trace!(
                        "[hooks] {event}: {} finished in {}ms",
                        run.label,
                        run.duration.as_millis()
                    );
                }
            }
        });
    }

    /// Ambient variables plus anything a `sessionStart` hook contributed.
    async fn env_for(&self, input: &HookInput) -> BTreeMap<String, String> {
        let mut env = exec::ambient_env(input);
        if let Some(session) = &input.session_id {
            if let Some(extra) = self.session_env.read().await.get(session) {
                env.extend(extra.clone());
            }
        }
        env
    }

    async fn absorb_session_env(&self, input: &HookInput, env: BTreeMap<String, String>) {
        let Some(session) = input.session_id.clone() else {
            log::debug!("[hooks] ignoring session env from a hook fired outside a session");
            return;
        };
        self.session_env
            .write()
            .await
            .entry(session)
            .or_default()
            .extend(env);
    }

    /// Charge one follow-up against a hook's budget, returning whether it may
    /// be honoured. A hook with no session id gets exactly one, since there is
    /// nowhere to keep a counter and an unbounded loop is the worse failure.
    async fn grant_followup(
        &self,
        input: &HookInput,
        definition: &HookDefinition,
        label: &str,
    ) -> bool {
        // Cursor's `loop_limit: null` means unlimited. serde cannot distinguish
        // an explicit null from an absent key in an `Option<u32>`, so the file
        // convention here is: absent → engine default, `0` → unlimited.
        let limit = match definition.loop_limit {
            Some(0) => return true,
            Some(limit) => limit,
            None => DEFAULT_LOOP_LIMIT,
        };
        let Some(session) = input.session_id.clone() else {
            return true;
        };
        let key = (session, label.to_string());
        let mut counts = self.loop_counts.write().await;
        let count = counts.entry(key).or_insert(0);
        if *count >= limit {
            return false;
        }
        *count += 1;
        true
    }

    /// Drop everything scoped to a finished session.
    pub async fn forget_session(&self, session_id: &str) {
        self.session_env.write().await.remove(session_id);
        self.loop_counts
            .write()
            .await
            .retain(|(session, _), _| session != session_id);
    }
}
