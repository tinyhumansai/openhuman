//! The host side of the queues' callbacks: `Deliver` and `ClassifySalience`.
//!
//! `Deliver` hands the host one due turn. For a trigger turn the host rebuilds
//! the envelope the event was queued with and runs triage on it:
//!
//! | triage outcome | delivery answer |
//! | --- | --- |
//! | a decision, applied (or still applying at the budget) | `Accepted` |
//! | a decision whose apply failed within the budget | `Retry` with the queue's back-off |
//! | `Deferred { defer_until_ms }` | `Retry { after_ms: defer_until_ms - now }` |
//! | `Terminal` | `Rejected` (dead-lettered, redrivable) |
//! | an error, or the triage timeout | `Retry` with the queue's back-off |
//!
//! So a deferred triage is redelivered when it asked to run again instead of
//! being dropped, and a failed one backs off and dead-letters after the lane's
//! attempts.
//!
//! The decision is applied while the delivery waits, within the delivery's
//! budget (what the module's 30 s callback deadline leaves after triage). An
//! apply that fails inside the budget is retried like a failed triage, so a
//! failed agent dispatch stays recoverable (and dead-letters, redrivable,
//! after the lane's attempts). An apply still running at the budget (a long
//! agent run, an approval that can wait ten minutes) is acknowledged and left
//! to finish in its own task: holding the turn longer would outlive the
//! callback deadline and have it delivered twice. A failure after that point
//! is reported, not retried.
//!
//! A digest turn (many events folded together) is triaged once, as one
//! envelope listing them. `ClassifySalience` runs the same triage for a
//! borderline event and maps the action to a salience class; the decision is
//! kept (for ten minutes) for the event's delivery so it is not triaged twice.
//!
//! A turn of a source configured under `[inbound.sources.*]` skips triage
//! altogether: it runs on the source's pinned agent ([`super::pinned`]), and
//! `ClassifySalience` answers `actionable` for its events without triaging.
//! A turn mixing sources (a digest that collected a spilled burst) is split:
//! each pinned source's events run on its agent, at most `max_batch` a turn,
//! under ids derived from the turn's, and only the rest is triaged.
//!
//! One decision per turn: the triage timeout stays under the module's
//! callback deadline (`[inbound] triage_timeout_ms`, at most 25 s), and a
//! turn that was already accepted is accepted again without triage, so a
//! redelivery that raced a slow answer never applies a second decision.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tinyeventqueues_bus::{
    ClassifySalienceRequest, ClassifySalienceResponse, DeliverRequest, DeliverResponse,
    DeliverStatus, InboundEvent, Lane, SalienceClass, Turn,
};

use super::events::{digest_envelope, envelope_of};
use super::pinned::{Delivered, Pinned};
use super::results::Results;
use crate::config::schema::{InboundConfig, InboundSourceConfig};
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::runtime::DispatchCtx;
use crate::triage::{TriageAction, TriageOutcome, TriageRun, TriageRunner, TriggerEnvelope};

/// How many classifications are kept for their delivery.
const CLASSIFIED_CAPACITY: usize = 256;

/// How long a classification is kept for its delivery.
const CLASSIFIED_TTL_MS: i64 = 10 * 60 * 1_000;

/// How many accepted turns are remembered against a redelivery.
const ACCEPTED_CAPACITY: usize = 1_024;

/// How long one delivery may take in all: under the module's 30 s callback
/// deadline, so its answer is never lost to the deadline.
const DELIVERY_BUDGET: Duration = Duration::from_secs(28);

/// The salience class of a triage action.
#[must_use]
pub fn class_of(action: TriageAction) -> SalienceClass {
    match action {
        TriageAction::Drop => SalienceClass::Noise,
        TriageAction::Acknowledge => SalienceClass::Fyi,
        TriageAction::React => SalienceClass::Actionable,
        TriageAction::Escalate => SalienceClass::Urgent,
    }
}

/// Decisions made while classifying, waiting for their event's delivery.
#[derive(Debug, Default)]
struct Classified {
    runs: HashMap<String, (TriageRun, i64)>,
    order: VecDeque<String>,
}

impl Classified {
    fn keep(&mut self, key: String, run: TriageRun, at_ms: i64) {
        if self.runs.insert(key.clone(), (run, at_ms)).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > CLASSIFIED_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.runs.remove(&oldest);
            }
        }
    }

    /// The decision kept for `key`, unless it is older than the TTL at `now_ms`.
    fn take(&mut self, key: &str, now_ms: i64) -> Option<TriageRun> {
        let (run, at_ms) = self.runs.remove(key)?;
        self.order.retain(|kept| kept != key);
        (now_ms - at_ms <= CLASSIFIED_TTL_MS).then_some(run)
    }
}

/// Turns already accepted, newest last.
#[derive(Debug, Default)]
struct Accepted {
    order: VecDeque<String>,
}

impl Accepted {
    fn contains(&self, turn_id: &str) -> bool {
        self.order.iter().any(|kept| kept == turn_id)
    }

    fn remember(&mut self, turn_id: &str) {
        if !self.contains(turn_id) {
            self.order.push_back(turn_id.to_owned());
        }
        while self.order.len() > ACCEPTED_CAPACITY {
            self.order.pop_front();
        }
    }
}

/// How applying a decision ended, as far as the delivery waited.
#[derive(Debug)]
enum Applied {
    /// Applied.
    Done,
    /// Still applying when the delivery's budget ran out; it finishes on its
    /// own.
    Running,
    /// Failed inside the budget.
    Failed(String),
}

/// What a delivery needs from its surroundings.
pub struct Deliverer {
    runner: Arc<dyn TriageRunner>,
    ctx: DispatchCtx,
    /// The triage timeout: given, or read from `[inbound]` on first use.
    timeout: tokio::sync::OnceCell<Duration>,
    now_ms: fn() -> i64,
    max_attempts: Box<dyn Fn(Lane) -> Option<u32> + Send + Sync>,
    budget: Duration,
    classified: Mutex<Classified>,
    accepted: Mutex<Accepted>,
    /// `[inbound]`: given, or read on first use, for its pinned sources.
    inbound: tokio::sync::OnceCell<Arc<InboundConfig>>,
    /// What runs pinned sources' turns: given, or built on first use.
    pinned: tokio::sync::OnceCell<Option<Pinned>>,
}

impl std::fmt::Debug for Deliverer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deliverer")
            .field("timeout", &self.timeout.get())
            .finish_non_exhaustive()
    }
}

fn system_now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Deliverer {
    /// A deliverer that triages with `runner` under `ctx`, giving one triage
    /// `timeout`, and reads each lane's attempts from the configured queues.
    /// It has no pinned sources unless given some ([`Self::with_inbound`]).
    #[must_use]
    pub fn new(runner: Arc<dyn TriageRunner>, ctx: DispatchCtx, timeout: Duration) -> Self {
        Self::build(runner, ctx, tokio::sync::OnceCell::new_with(Some(timeout)))
            .with_inbound(InboundConfig::default())
    }

    /// A deliverer whose triage timeout is `[inbound] triage_timeout_ms`, read
    /// from `ctx`'s configuration on the first callback. Loading it when the
    /// callbacks are served instead would initialise a config file on a host
    /// that never configured one. Its pinned sources (`[inbound.sources.*]`)
    /// are read the same way.
    #[must_use]
    pub fn configured(runner: Arc<dyn TriageRunner>, ctx: DispatchCtx) -> Self {
        Self::build(runner, ctx, tokio::sync::OnceCell::new())
    }

    fn build(
        runner: Arc<dyn TriageRunner>,
        ctx: DispatchCtx,
        timeout: tokio::sync::OnceCell<Duration>,
    ) -> Self {
        Self {
            runner,
            ctx,
            timeout,
            now_ms: system_now_ms,
            max_attempts: Box::new(|lane| super::settings::current().max_attempts(lane)),
            budget: DELIVERY_BUDGET,
            classified: Mutex::new(Classified::default()),
            accepted: Mutex::new(Accepted::default()),
            inbound: tokio::sync::OnceCell::new(),
            pinned: tokio::sync::OnceCell::new(),
        }
    }

    /// Read pinned sources from `inbound` instead of the configuration.
    #[must_use]
    pub fn with_inbound(mut self, inbound: InboundConfig) -> Self {
        self.inbound = tokio::sync::OnceCell::new_with(Some(Arc::new(inbound)));
        self
    }

    /// Run pinned sources' turns with `pinned` instead of the production
    /// agent and workspace results store.
    #[must_use]
    pub fn with_pinned(mut self, pinned: Pinned) -> Self {
        self.pinned = tokio::sync::OnceCell::new_with(Some(Some(pinned)));
        self
    }

    /// Use `now_ms` for the clock (tests).
    #[must_use]
    pub fn with_clock(mut self, now_ms: fn() -> i64) -> Self {
        self.now_ms = now_ms;
        self
    }

    /// Bound one delivery by `budget` instead of [`DELIVERY_BUDGET`] (tests).
    #[must_use]
    pub fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    /// Read each lane's attempts from `max_attempts` (tests).
    #[must_use]
    pub fn with_max_attempts(
        mut self,
        max_attempts: impl Fn(Lane) -> Option<u32> + Send + Sync + 'static,
    ) -> Self {
        self.max_attempts = Box::new(max_attempts);
        self
    }

    /// The triage timeout, read from `[inbound]` on first use when not given.
    async fn timeout(&self) -> Duration {
        *self
            .timeout
            .get_or_init(|| async {
                match self.ctx.load_config().await {
                    Ok(config) => config.inbound.triage_timeout(),
                    Err(error) => {
                        tracing::debug!(error = %error, "[inbound] config unavailable; default triage timeout");
                        crate::config::schema::InboundConfig::default().triage_timeout()
                    }
                }
            })
            .await
    }

    /// `[inbound]`, read from the configuration on first use when not given.
    async fn inbound(&self) -> Arc<InboundConfig> {
        Arc::clone(
            self.inbound
                .get_or_init(|| async {
                    match self.ctx.load_config().await {
                        Ok(config) => Arc::new(config.inbound),
                        Err(error) => {
                            tracing::debug!(error = %error, "[inbound] config unavailable; no pinned sources");
                            Arc::new(InboundConfig::default())
                        }
                    }
                })
                .await,
        )
    }

    /// The pinned source every event of `turn` comes from, with its
    /// configuration, when there is one.
    async fn pinned_source(
        &self,
        events: &[InboundEvent],
    ) -> Option<(String, InboundSourceConfig)> {
        let first = events.first()?;
        if !events.iter().all(|event| event.source == first.source) {
            return None;
        }
        let inbound = self.inbound().await;
        let source = inbound.source(&first.source).ok()?.clone();
        Some((first.source.clone(), source))
    }

    /// `turn`'s events of pinned sources, one group per source and
    /// conversation key, as turns of at most the source's `max_batch` events,
    /// and the turn of the other events (`None` when there are none). Each
    /// group is oldest first (event id breaks ties), and a part's id names
    /// its first and last event, so a redelivery derives the same ids and a
    /// changed `max_batch` derives new ones instead of reusing a finished id
    /// for a different set of events.
    async fn split_pinned(
        &self,
        turn: &Turn,
    ) -> (Vec<(Turn, String, InboundSourceConfig)>, Option<Turn>) {
        let inbound = self.inbound().await;
        let mut pinned: BTreeMap<(String, String), (InboundSourceConfig, Vec<InboundEvent>)> =
            BTreeMap::new();
        let mut rest = Vec::new();
        for event in &turn.events {
            match inbound.source(&event.source) {
                Ok(source) => pinned
                    .entry((event.source.clone(), event.conversation_key.clone()))
                    .or_insert_with(|| (source.clone(), Vec::new()))
                    .1
                    .push(event.clone()),
                Err(_) => rest.push(event.clone()),
            }
        }
        let mut parts = Vec::new();
        for ((name, key), (source, mut events)) in pinned {
            events.sort_by(|a, b| {
                (a.received_at_ms, &a.event_id).cmp(&(b.received_at_ms, &b.event_id))
            });
            let max = usize::try_from(source.max_batch.max(1)).unwrap_or(usize::MAX);
            for chunk in events.chunks(max) {
                let (first, last) = (&chunk[0].event_id, &chunk[chunk.len() - 1].event_id);
                let mut part = turn.clone();
                part.turn_id = format!("{}:{name}:{key}:{first}..{last}", turn.turn_id);
                part.events = chunk.to_vec();
                parts.push((part, name.clone(), source.clone()));
            }
        }
        let rest = (!rest.is_empty()).then(|| {
            let mut left = turn.clone();
            left.events = rest;
            left
        });
        (parts, rest)
    }

    /// What runs pinned turns: given, or the production agent over the
    /// workspace's results store.
    async fn pinned(&self) -> Option<Pinned> {
        self.pinned
            .get_or_init(|| async {
                let config = match self.ctx.load_config().await {
                    Ok(config) => config,
                    Err(error) => {
                        tracing::warn!(error = %error, "[inbound] config unavailable; pinned turns cannot run");
                        return None;
                    }
                };
                match Results::open(&config) {
                    Ok(results) => Some(Pinned::production(results)),
                    Err(error) => {
                        tracing::warn!(error = %error, "[inbound] results store unavailable; pinned turns cannot run");
                        None
                    }
                }
            })
            .await
            .clone()
    }

    /// Deliver a turn of pinned source `name`.
    async fn deliver_pinned(
        &self,
        turn: &Turn,
        name: &str,
        source: &InboundSourceConfig,
        within: Duration,
    ) -> DeliverStatus {
        let Some(pinned) = self.pinned().await else {
            return self.retry(turn, None, "pinned turns cannot run here");
        };
        match pinned.deliver(&self.ctx, turn, name, source, within).await {
            Delivered::Done | Delivered::Running => {
                self.accepted().remember(&turn.turn_id);
                DeliverStatus::Accepted
            }
            Delivered::Failed(error) => {
                tracing::warn!(turn = %turn.turn_id, source = %name, attempt = turn.attempt, error = %error, "[inbound] pinned agent failed");
                if self.exhausted(turn) {
                    pinned.fail(&turn.turn_id, &error);
                }
                self.retry(turn, None, "pinned agent failed")
            }
        }
    }

    fn classified(&self) -> std::sync::MutexGuard<'_, Classified> {
        self.classified
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn accepted(&self) -> std::sync::MutexGuard<'_, Accepted> {
        self.accepted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Classify a borderline event by triaging it.
    ///
    /// # Errors
    ///
    /// The event is not a trigger, or triage did not decide in time; the
    /// queues then keep the class its score gave it.
    pub async fn classify(
        &self,
        request: ClassifySalienceRequest,
    ) -> Result<ClassifySalienceResponse, String> {
        let event = request.event;
        if self
            .pinned_source(std::slice::from_ref(&event))
            .await
            .is_some()
        {
            // A pinned source's events always reach its agent: no triage.
            return Ok(ClassifySalienceResponse::new(SalienceClass::Actionable));
        }
        let envelope = envelope_of(&event).ok_or("not an inbound trigger")?;
        let timeout = self.timeout().await;
        match tokio::time::timeout(timeout, self.runner.run(&self.ctx, &envelope)).await {
            Ok(Ok(TriageOutcome::Decision(run))) => {
                let class = class_of(run.decision.action);
                tracing::debug!(
                    source = %event.source,
                    event_id = %event.event_id,
                    score = request.score,
                    class = ?class,
                    "[inbound] classified"
                );
                self.classified()
                    .keep(event.dedupe_key(), run, (self.now_ms)());
                Ok(ClassifySalienceResponse::new(class))
            }
            Ok(Ok(_)) => Err("triage did not decide".to_owned()),
            Ok(Err(error)) => Err(format!("triage failed: {error}")),
            Err(_) => Err("triage timed out".to_owned()),
        }
    }

    /// Deliver one due turn.
    pub async fn deliver(&self, request: DeliverRequest) -> DeliverResponse {
        let started = tokio::time::Instant::now();
        let turn = request.turn;
        if self.accepted().contains(&turn.turn_id) {
            // The module redelivered a turn whose decision is already being
            // applied (its answer missed the callback deadline): say so again
            // rather than deciding and acting a second time.
            tracing::info!(
                turn = %turn.turn_id,
                attempt = turn.attempt,
                "[inbound] turn already accepted; not triaging it again"
            );
            return DeliverResponse::new(DeliverStatus::Accepted);
        }
        if let Some((name, source)) = self.pinned_source(&turn.events).await {
            let remaining = self.budget.saturating_sub(started.elapsed());
            let status = self.deliver_pinned(&turn, &name, &source, remaining).await;
            return DeliverResponse::new(status);
        }
        let (parts, rest) = self.split_pinned(&turn).await;
        for (index, (part, name, source)) in parts.iter().enumerate() {
            if index > 0 && started.elapsed() >= self.budget {
                // Don't start another part with no budget left; the redelivery
                // skips the finished parts and resumes here.
                return DeliverResponse::new(self.retry(
                    &turn,
                    None,
                    "delivery budget spent on pinned parts",
                ));
            }
            let remaining = self.budget.saturating_sub(started.elapsed());
            let status = self.deliver_pinned(part, name, source, remaining).await;
            // Retried before the rest is triaged, so the rest is decided once;
            // the parts already finished are not run again on redelivery.
            if status != DeliverStatus::Accepted {
                return DeliverResponse::new(status);
            }
        }
        if !parts.is_empty() && rest.is_some() && started.elapsed() >= self.budget {
            // The pinned parts used the whole budget: triage the rest on the
            // redelivery, where the finished parts are skipped.
            return DeliverResponse::new(self.retry(
                &turn,
                None,
                "delivery budget spent on pinned parts",
            ));
        }
        let turn = match rest {
            Some(rest) => rest,
            None if parts.is_empty() => turn,
            None => {
                self.accepted().remember(&turn.turn_id);
                return DeliverResponse::new(DeliverStatus::Accepted);
            }
        };
        let (envelope, first) = match self.envelope(&turn) {
            Ok(found) => found,
            Err(detail) => {
                tracing::warn!(turn = %turn.turn_id, detail = %detail, "[inbound] undeliverable turn");
                self.dead(&turn, &detail);
                return DeliverResponse::new(DeliverStatus::Rejected { detail });
            }
        };
        let now_ms = (self.now_ms)();
        let cached = first
            .as_ref()
            .and_then(|event| self.classified().take(&event.dedupe_key(), now_ms));
        let outcome = match cached {
            Some(run) => Ok(Ok(TriageOutcome::Decision(run))),
            None => {
                // Within what is left of the delivery's budget.
                let timeout = self
                    .timeout()
                    .await
                    .min(self.budget.saturating_sub(started.elapsed()));
                tokio::time::timeout(timeout, self.runner.run(&self.ctx, &envelope)).await
            }
        };
        let status = match outcome {
            Ok(Ok(TriageOutcome::Decision(run))) => {
                tracing::info!(
                    turn = %turn.turn_id,
                    label = %envelope.display_label,
                    action = %run.decision.action.as_str(),
                    attempt = turn.attempt,
                    "[inbound] delivered; applying the decision"
                );
                let remaining = self.budget.saturating_sub(started.elapsed());
                match self.apply(run, envelope, remaining).await {
                    Applied::Done | Applied::Running => {
                        self.accepted().remember(&turn.turn_id);
                        DeliverStatus::Accepted
                    }
                    Applied::Failed(error) => {
                        self.retry(&turn, None, &format!("apply failed: {error}"))
                    }
                }
            }
            Ok(Ok(TriageOutcome::Deferred {
                defer_until_ms,
                reason,
            })) => {
                let wait = u64::try_from(defer_until_ms - now_ms).unwrap_or(0);
                tracing::info!(
                    turn = %turn.turn_id,
                    label = %envelope.display_label,
                    defer_until_ms,
                    after_ms = wait,
                    reason = %reason,
                    "[inbound] triage deferred; retrying then"
                );
                for event in &turn.events {
                    BUS.publish(DomainEvent::InboundDeferred {
                        source: event.source.clone(),
                        event_id: event.event_id.clone(),
                        defer_until_ms,
                        reason: reason.clone(),
                    });
                }
                self.retry(&turn, Some(wait), &reason)
            }
            Ok(Ok(TriageOutcome::Terminal { reason })) => {
                tracing::warn!(
                    turn = %turn.turn_id,
                    label = %envelope.display_label,
                    reason = %reason,
                    "[inbound] triage terminal; dead-lettering"
                );
                self.dead(&turn, &reason);
                DeliverStatus::Rejected { detail: reason }
            }
            Ok(Err(error)) => {
                let detail = format!(
                    "[inbound] triage failed (label={}): {error:#}",
                    envelope.display_label
                );
                crate::core::observability::report_error_or_expected(
                    detail.as_str(),
                    "inbound",
                    "deliver_triage",
                    &[("label", envelope.display_label.as_str())],
                );
                self.retry(&turn, None, "triage failed")
            }
            Err(_) => {
                tracing::warn!(
                    turn = %turn.turn_id,
                    label = %envelope.display_label,
                    timeout_ms = self.timeout.get().map_or(0, |t| t.as_millis() as u64),
                    "[inbound] triage timed out; retrying"
                );
                self.retry(&turn, None, "triage timed out")
            }
        };
        DeliverResponse::new(status)
    }

    /// The envelope to triage for `turn`, and its one event when it has one.
    fn envelope(&self, turn: &Turn) -> Result<(TriggerEnvelope, Option<InboundEvent>), String> {
        match turn.events.as_slice() {
            [] => Err("turn carries no events".to_owned()),
            [event] if !turn.is_digest() => envelope_of(event)
                .map(|envelope| (envelope, Some(event.clone())))
                .ok_or_else(|| format!("{} is not an inbound trigger", event.source)),
            _ => Ok((digest_envelope(turn), None)),
        }
    }

    /// Apply `run` in its own task and wait for it for at most `within`.
    async fn apply(&self, run: TriageRun, envelope: TriggerEnvelope, within: Duration) -> Applied {
        let runner = Arc::clone(&self.runner);
        let ctx = self.ctx.clone();
        let label = envelope.display_label.clone();
        let mut task = self.ctx.spawn(async move {
            let applied = runner.apply(&ctx, run, &envelope).await;
            if let Err(error) = &applied {
                // Reported here, wherever the delivery stands: inside the
                // budget the turn is retried, after it this is the only trace.
                let detail = format!(
                    "[inbound] apply_decision failed (label={}): {error:#}",
                    envelope.display_label
                );
                crate::core::observability::report_error_or_expected(
                    detail.as_str(),
                    "inbound",
                    "apply_decision",
                    &[("label", envelope.display_label.as_str())],
                );
            }
            applied.map_err(|error| format!("{error:#}"))
        });
        match tokio::time::timeout(within, &mut task).await {
            Ok(Ok(Ok(()))) => Applied::Done,
            Ok(Ok(Err(error))) => Applied::Failed(error),
            Ok(Err(join)) => Applied::Failed(format!("apply task ended: {join}")),
            Err(_) => {
                tracing::info!(
                    label = %label,
                    "[inbound] decision still applying at the delivery budget; acknowledging"
                );
                Applied::Running
            }
        }
    }

    /// Ask the queues to deliver `turn` again; when this was its last attempt
    /// the queues dead-letter it instead, so say so.
    fn retry(&self, turn: &Turn, after_ms: Option<u64>, why: &str) -> DeliverStatus {
        if self.exhausted(turn) {
            self.dead(turn, &format!("{why}; attempts exhausted"));
        }
        DeliverStatus::Retry { after_ms }
    }

    /// Whether `turn` is on its lane's last attempt.
    fn exhausted(&self, turn: &Turn) -> bool {
        (self.max_attempts)(turn.lane).is_some_and(|max| turn.attempt >= max)
    }

    fn dead(&self, turn: &Turn, detail: &str) {
        for event in &turn.events {
            BUS.publish(DomainEvent::InboundDeadLettered {
                source: event.source.clone(),
                event_id: event.event_id.clone(),
                attempt: turn.attempt,
                detail: detail.to_owned(),
            });
        }
    }
}

#[cfg(test)]
#[path = "deliver_tests.rs"]
mod tests;
