//! OpenHuman adapters for TinyMemory's runtime-neutral turn hooks.

use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde::Serialize;
use tinymemory_api::{Hit, ObservedActor, ThreadWindow, ToolCallRef, Turn};
use tinymemory_tools::layout::identity::ResolvedIdentity;
use tinymemory_tools::lifecycle::turn::{
    self as native_turn, CitationSink, Clock, EngineResolver, JobsSink, LoggedToolCall,
    PreTurnOptions, TurnHooks as NativeTurnHooks, TurnPack as NativeTurnPack,
};
use tinymemory_tools::{AgentMemory, BackgroundJob, Compaction, PostTurn, PreTurn};

use crate::config::Config;
use crate::core::runtime::{CoreContext, DispatchCtx};
use crate::memory::engine;

pub use native_turn::{user_turn_index, CLOSE_TAG, MAX_TOOL_LINE_CHARS, OPEN_TAG};

/// A native turn pack with the citation view used by OpenHuman's chat UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnPack {
    /// TinyMemory's authoritative pack and refusal state.
    #[serde(flatten)]
    pub native: NativeTurnPack,
    /// The citation view attached to the chat reply, not serialized with the pack.
    #[serde(skip)]
    pub citations: Vec<crate::web_chat::TurnCitation>,
}

impl TurnPack {
    /// Adapts generic TinyMemory hits to the citation shape used by web chat.
    #[must_use]
    pub fn from_native(native: NativeTurnPack) -> Self {
        let citations = native
            .citations
            .iter()
            .map(crate::web_chat::TurnCitation::from)
            .collect();
        Self { native, citations }
    }

    /// The block as the model sees it.
    #[must_use]
    pub fn injection(&self) -> String {
        self.native.injection()
    }
}

impl std::ops::Deref for TurnPack {
    type Target = NativeTurnPack;

    fn deref(&self) -> &Self::Target {
        &self.native
    }
}

impl std::ops::DerefMut for TurnPack {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.native
    }
}

/// One turn's memory, carried on its run context.
#[derive(Debug, Clone)]
pub struct MemoryTurn {
    /// The config the session runs under.
    pub config: Arc<Config>,
    /// Who the turn runs as.
    pub identity: ResolvedIdentity,
    /// The conversation thread.
    pub thread_id: String,
    /// The pack the turn was given, if any.
    pub pack: Option<TurnPack>,
    /// The first user turn still verbatim in this turn's prompt.
    pub in_prompt_from: Arc<tinymemory_tools::lifecycle::window::InPromptFrom>,
    /// The turn's core context, held weakly so a finished tenant's lease is not retained.
    pub core: Option<Weak<CoreContext>>,
}

impl MemoryTurn {
    /// The thread's turns still verbatim in the prompt.
    #[must_use]
    pub fn in_prompt(&self) -> ThreadWindow {
        ThreadWindow {
            thread_id: self.thread_id.clone(),
            from_turn: self.in_prompt_from.get(),
        }
    }

    /// The dispatch context for this turn, falling back to the caller's context.
    #[must_use]
    pub fn dispatch_ctx(&self) -> DispatchCtx {
        DispatchCtx::from_weak(self.core.as_ref())
    }
}

/// A tool call paired with the one-line result included in the logged reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallSummary {
    /// The tool's name.
    pub name: String,
    /// The call id.
    pub id: Option<String>,
    /// The start of its result, if it produced one.
    pub result: Option<String>,
}

fn native_tool_calls(calls: &[ToolCallSummary]) -> Vec<LoggedToolCall> {
    calls
        .iter()
        .map(|call| LoggedToolCall {
            call: ToolCallRef {
                name: call.name.clone(),
                id: call.id.clone(),
            },
            result: call.result.clone(),
        })
        .collect()
}

/// Formats the reply and one bounded result line per tool call.
#[must_use]
pub fn logged_reply(text: &str, calls: &[ToolCallSummary]) -> String {
    native_turn::logged_reply(text, &native_tool_calls(calls))
}

struct HostEngineResolver {
    config: Arc<Config>,
    identity: ResolvedIdentity,
}

#[async_trait]
impl EngineResolver for HostEngineResolver {
    async fn resolve(&self) -> tinymemory_api::Result<AgentMemory> {
        let bound = engine::resolve(&self.config).await.engine()?;
        super::agent_memory_on(bound.engine, &self.config, &self.identity)
    }
}

struct TokioClock {
    origin: tokio::time::Instant,
}

impl Clock for TokioClock {
    fn now(&self) -> Duration {
        tokio::time::Instant::now().saturating_duration_since(self.origin)
    }

    fn sleep_until(&self, deadline: Duration) -> BoxFuture<'static, ()> {
        let deadline = self.origin + deadline;
        Box::pin(async move { tokio::time::sleep_until(deadline).await })
    }
}

struct HostJobsSink {
    config: Arc<Config>,
    root: tinymemory_api::Namespace,
}

#[async_trait]
impl JobsSink for HostJobsSink {
    async fn enqueue(&self, jobs: Vec<BackgroundJob>) {
        super::jobs::enqueue(&self.config, &self.root, jobs).await;
    }
}

struct HostCitationSink {
    ctx: DispatchCtx,
    thread_id: String,
}

impl CitationSink for HostCitationSink {
    fn deliver(&self, citations: Vec<Hit>) {
        crate::web_chat::citations::record_pack_citations(
            &self.ctx,
            &self.thread_id,
            citations
                .iter()
                .map(crate::web_chat::TurnCitation::from)
                .collect(),
        );
    }
}

fn turn_hooks(
    ctx: &DispatchCtx,
    config: &Arc<Config>,
    identity: &ResolvedIdentity,
    thread_id: &str,
) -> Arc<NativeTurnHooks> {
    let resolver = Arc::new(HostEngineResolver {
        config: config.clone(),
        identity: identity.clone(),
    });
    let clock = Arc::new(TokioClock {
        origin: tokio::time::Instant::now(),
    });
    let date_hint_model = Some(super::date_hint::model(ctx.clone(), config.clone()));
    let jobs = Arc::new(HostJobsSink {
        config: config.clone(),
        root: identity.root().clone(),
    });
    let citations = Arc::new(HostCitationSink {
        ctx: ctx.clone(),
        thread_id: thread_id.to_owned(),
    });
    Arc::new(NativeTurnHooks::new(
        resolver,
        clock,
        date_hint_model,
        jobs,
        citations,
    ))
}

/// Runs one bounded pre-turn operation through TinyMemory's lifecycle hooks.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn pre_turn_work(
    ctx: &DispatchCtx,
    config: &Config,
    identity: &ResolvedIdentity,
    thread_id: String,
    turn_index: u32,
    user_text: String,
    in_prompt_from: u32,
    at: DateTime<Utc>,
    resumed_after_compaction: bool,
    observed_actor: Option<ObservedActor>,
    channel: Option<String>,
) -> Option<TurnPack> {
    let config = Arc::new(config.clone());
    let mut turn = PreTurn::new(thread_id.clone(), turn_index, user_text);
    turn.in_prompt_from = in_prompt_from;
    turn.at = Some(at);
    turn.resumed = resumed_after_compaction;
    turn.observed_actor = observed_actor;
    turn.channel = channel;
    let logging = config.memory.conversations.enabled;
    let recall = identity.recall;
    let hooks = turn_hooks(ctx, &config, identity, &thread_id);
    let options = PreTurnOptions {
        logging,
        recall,
        timeout: Duration::from_millis(config.memory.recall.pre_turn_timeout_ms.max(1)),
        date_hint: logging && recall && config.memory.recall.date_hint,
        zone: Some(config.time_zone()),
        date_hint_margin: Duration::from_millis(100),
    };
    hooks
        .pre_turn_background_work(turn, options)
        .await
        .map(TurnPack::from_native)
}

/// Queues citations through TinyMemory's explicit shown-pack delivery hook.
pub(crate) fn deliver_citations(
    ctx: &DispatchCtx,
    config: &Config,
    identity: &ResolvedIdentity,
    thread_id: &str,
    pack: &TurnPack,
) {
    let hooks = turn_hooks(ctx, &Arc::new(config.clone()), identity, thread_id);
    hooks.deliver_citations(&pack.native);
}

/// Logs a committed reply and awaits queuing the jobs returned by TinyMemory.
pub async fn post_turn(
    ctx: &DispatchCtx,
    config: &Config,
    identity: &ResolvedIdentity,
    thread_id: String,
    turn_index: u32,
    assistant_text: String,
    tool_calls: Vec<ToolCallSummary>,
    at: DateTime<Utc>,
    channel: Option<String>,
) {
    if !config.memory.conversations.enabled {
        return;
    }
    let native_calls = native_tool_calls(&tool_calls);
    let text = native_turn::logged_reply(&assistant_text, &native_calls);
    if text.is_empty() {
        return;
    }
    let mut turn = PostTurn::new(thread_id.clone(), turn_index, text);
    turn.channel = channel;
    turn.at = Some(at);
    turn.tool_calls = native_calls.into_iter().map(|call| call.call).collect();
    turn_hooks(ctx, &Arc::new(config.clone()), identity, &thread_id)
        .post_turn(turn, true)
        .await;
}

/// Direct compaction recall, owned and cancelled by the background worker.
pub(crate) async fn compaction_work(
    ctx: &DispatchCtx,
    config: &Config,
    identity: &ResolvedIdentity,
    thread_id: &str,
    dropped: Vec<Turn>,
) -> Option<TurnPack> {
    let hooks = turn_hooks(ctx, &Arc::new(config.clone()), identity, thread_id);
    hooks
        .compaction_background_work(
            Compaction {
                thread_id: thread_id.to_owned(),
                dropped,
                focus: None,
            },
            identity.recall,
        )
        .await
        .map(TurnPack::from_native)
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
