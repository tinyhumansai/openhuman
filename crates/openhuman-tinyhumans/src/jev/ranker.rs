//! [`TinyHumansJevRanker`]: a `ToolRanker` that resolves the process's
//! TinyHumans credential on every search and ranks through Jev.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Mutex,
};

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use openhuman_embed::__host::agent::tinyagents::discovery::{
    embedding_provider_is_usable, embedding_tool_ranker_in,
};
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::inference::embedding_host::SharedEmbeddingModel;
use openhuman_embed::__host::security::approval::ApprovalChatContext;
use tinyjevclient::Client;
use tinytools::{RankCandidate, RankContext, RankError, RankHit, ToolRanker};
use tinytools_jev::{JevRanker, JevRankerConfig, JevStrategy};

use super::evaluator::TinyJevEvaluator;
use super::route::{self, EnvLookup};

/// How the ranker reads the config a search runs under. The default is the
/// core's own read path (the embedder's config when one is bound, else the
/// process-global load); a test hands in a fixed one.
pub type ConfigLoader =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<Config, String>> + Send>> + Send + Sync>;

/// How the ranker builds the embedding provider its retriever searches with.
/// The second argument is the chat the search runs for: a cloud provider shows
/// its egress disclosure there. The default is the core's configured provider;
/// a test hands in one that records the chat.
pub type EmbedderFactory =
    Arc<dyn Fn(&Config, Option<ApprovalChatContext>) -> SharedEmbeddingModel + Send + Sync>;

/// The core's configured embedding provider, routed to the chat when there is one.
fn default_embedder() -> EmbedderFactory {
    Arc::new(|config, chat| {
        openhuman_embed::__host::inference::embedding_host::default_embedding_provider_for_chat(
            config, chat,
        )
    })
}

/// Where the ranker reads the two env-provided Jev keys. The default is the
/// process environment; a test hands in a fixed table.
pub type EnvLoader = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// A [`JevRanker`] bound to the Jev route the config selects
/// (`agent.tool_search.jev_route`) and the credential for it, resolved at
/// search time.
pub struct TinyHumansJevRanker {
    config: JevRankerConfig,
    load_config: ConfigLoader,
    env: EnvLoader,
    embedder: EmbedderFactory,
    /// Deadline for one evaluation. Measured through the TinyHumans proxy
    /// (2026-09) one evaluation takes 0.7–1.9 s at p50 and the family
    /// strategy runs its second-stage evaluations concurrently, so six
    /// seconds bounds a slow search well above the norm while still turning
    /// a stalled proxy into a BM25 fallback inside the turn.
    deadline: Duration,
    cached: Mutex<Option<Cached>>,
}

/// Default per-evaluation deadline; see `TinyHumansJevRanker::deadline`.
const DEFAULT_DEADLINE: Duration = Duration::from_secs(6);

struct Cached {
    fingerprint: u64,
    ranker: JevRanker,
    evaluator: Arc<dyn tinytools_jev::JevEvaluator>,
    /// The retriever inside `ranker`, kept so its catalogue embeddings
    /// survive a credential change.
    retriever: Arc<dyn ToolRanker>,
}

impl std::fmt::Debug for TinyHumansJevRanker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TinyHumansJevRanker")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Default for TinyHumansJevRanker {
    fn default() -> Self {
        Self::new()
    }
}

impl TinyHumansJevRanker {
    /// A ranker with the product defaults: the process's embedding provider
    /// retrieves the top 20 tools by meaning, one Jev evaluation decides
    /// (`RetrieveThenDecide`), 6 s deadline per evaluation. Without a usable
    /// embedding provider the search does not run and the harness ranks
    /// with BM25 alone — a lexical shortlist would cap Jev at BM25's recall,
    /// which the bench measured at 70% on the Composio catalogue.
    pub fn new() -> Self {
        Self::with_config(JevRankerConfig::new().with_strategy(JevStrategy::RetrieveThenDecide))
    }

    /// A ranker with an explicit `tinytools-jev` configuration.
    pub fn with_config(config: JevRankerConfig) -> Self {
        Self {
            config,
            load_config: Arc::new(|| {
                Box::pin(openhuman_embed::__host::config::ops::load_config_with_timeout())
            }),
            env: Arc::new(route::process_env),
            embedder: default_embedder(),
            deadline: DEFAULT_DEADLINE,
            cached: Mutex::new(None),
        }
    }

    /// Reads `TYPESAFE_API_KEY` / `OPENROUTER_API_KEY` through `env` instead of
    /// the process environment.
    pub fn with_env_loader(mut self, env: EnvLoader) -> Self {
        self.env = env;
        self
    }

    /// Builds the retriever's embedding provider through `embedder` instead of
    /// the core's configured one.
    pub fn with_embedder_factory(mut self, embedder: EmbedderFactory) -> Self {
        self.embedder = embedder;
        self
    }

    /// Sets the per-evaluation deadline.
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Reads the config through `loader` instead of the core's read path.
    pub fn with_config_loader(mut self, loader: ConfigLoader) -> Self {
        self.load_config = loader;
        self
    }

    /// The `JevRanker` for the current credential and backend, built or
    /// reused.
    ///
    /// Reads the config and credential fresh each time: the cost is a config
    /// load, which a `tool_search` (one model round trip plus a network call)
    /// dwarfs, and the benefit is that sign-in, sign-out and a backend URL
    /// change are all honoured by the next search.
    async fn current(&self, chat: Option<ApprovalChatContext>) -> Result<JevRanker, RankError> {
        let config = (self.load_config)()
            .await
            .map_err(|error| RankError::Backend {
                reason: format!("config unavailable: {error}"),
            })?;
        let env: EnvLookup<'_> = &*self.env;
        let resolved = route::resolve(&config, env)?;
        let base_url = resolved.client.base_url.clone();
        let route_label = resolved.route.label();
        let fingerprint = fingerprint(&format!("{route_label}:{}", resolved.secret), &base_url);

        let mut cached = self
            .cached
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cached
            .as_ref()
            .filter(|entry| entry.fingerprint == fingerprint)
        {
            return Ok(match chat {
                None => entry.ranker.clone(),
                // The cached retriever was built without a chat, so its cloud
                // disclosures reach nobody. A search made in a chat gets a
                // retriever routed to that chat; the catalogue embeddings it
                // reads live in the durable cache, not in the retriever.
                Some(chat) => self.routed(entry, &config, chat)?,
            });
        }
        let client_config = resolved.client;
        let client = Client::new(client_config)
            .map_err(|error| RankError::invalid_input(error.to_string()))?;
        let evaluator: Arc<dyn tinytools_jev::JevEvaluator> =
            Arc::new(TinyJevEvaluator::new(client).with_deadline(self.deadline));
        // The retriever is the process's embedding provider when it can
        // embed (the same one memory recall uses), so a family larger than
        // one Jev Choice is cut by meaning, not by shared words. Reused
        // across rebuilds so the catalogue is embedded once per process.
        let retriever: Arc<dyn ToolRanker> = match cached.as_ref() {
            Some(entry) => entry.retriever.clone(),
            None => retriever_for(&config, None, &self.embedder)?,
        };
        let ranker = JevRanker::new(
            evaluator.clone(),
            self.config.clone().with_retriever(retriever.clone()),
        );
        log::info!(
            "[tool-search] jev ranker bound to route={route_label} backend {} ({})",
            openhuman_embed::__host::util::log_redaction::scrub_secrets(&base_url),
            if fingerprint_changed(cached.as_ref(), fingerprint) {
                "credential or backend changed"
            } else {
                "first search"
            }
        );
        *cached = Some(Cached {
            fingerprint,
            ranker: ranker.clone(),
            evaluator,
            retriever,
        });
        match (chat, cached.as_ref()) {
            (Some(chat), Some(entry)) => self.routed(entry, &config, chat),
            _ => Ok(ranker),
        }
    }

    /// `entry`'s ranker, retrieving through a provider whose cloud
    /// disclosures go to `chat`.
    fn routed(
        &self,
        entry: &Cached,
        config: &Config,
        chat: ApprovalChatContext,
    ) -> Result<JevRanker, RankError> {
        let retriever = retriever_for(config, Some(chat), &self.embedder)?;
        Ok(JevRanker::new(
            entry.evaluator.clone(),
            self.config.clone().with_retriever(retriever),
        ))
    }
}

/// The semantic retriever for `config`'s embedding provider.
///
/// A provider that cannot embed (`none`) is an error, not a BM25 substitute:
/// the harness answers the search with its own BM25 catalogue in that case,
/// and a Jev decision over a lexical shortlist would only add a network
/// round trip to the same recall.
fn retriever_for(
    config: &Config,
    chat: Option<ApprovalChatContext>,
    embedder: &EmbedderFactory,
) -> Result<Arc<dyn ToolRanker>, RankError> {
    let provider = embedder(config, chat);
    if !embedding_provider_is_usable(provider.as_ref()) {
        log::info!(
            "[tool-search] embedding provider `{}` cannot embed; jev search disabled, bm25 answers",
            provider.name()
        );
        return Err(RankError::backend(format!(
            "no usable embedding provider (`{}`); jev search disabled",
            provider.name()
        )));
    }
    log::info!(
        "[tool-search] retrieving with embeddings ({} / {})",
        provider.name(),
        provider.model_id()
    );
    Ok(Arc::new(embedding_tool_ranker_in(
        provider,
        &config.workspace_dir,
    )))
}

fn fingerprint(secret: &str, base_url: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    secret.hash(&mut hasher);
    base_url.hash(&mut hasher);
    hasher.finish()
}

fn fingerprint_changed(cached: Option<&Cached>, fingerprint: u64) -> bool {
    cached.is_some_and(|entry| entry.fingerprint != fingerprint)
}

#[async_trait::async_trait]
impl ToolRanker for TinyHumansJevRanker {
    fn kind(&self) -> &'static str {
        JevRanker::KIND
    }

    async fn rank(
        &self,
        intent: &str,
        context: &RankContext,
        candidates: &[RankCandidate],
        limit: usize,
    ) -> Result<Vec<RankHit>, RankError> {
        // The chat the search runs for, from the turn's rank context: a cloud
        // embedder discloses the intent there.
        let chat = context.chat.as_ref().map(|chat| ApprovalChatContext {
            thread_id: chat.thread_id.clone(),
            client_id: chat.client_id.clone(),
            request_id: chat.request_id.clone(),
        });
        let ranker = self.current(chat).await?;
        let ranking = ranker
            .rank_detailed(intent, context, candidates, limit)
            .await?;
        log::debug!(
            "[tool-search] jev ranked {} of {} shown (families={:?} choice_confidence={:.2} needs_tool={:?} none={:.2} latency_ms={} attempts={} input_tokens={:?})",
            ranking.hits.len(),
            ranking.shortlisted,
            ranking.families,
            ranking.choice_confidence,
            ranking.needs_tool,
            ranking.none_probability,
            ranking.latency.as_millis(),
            ranking.attempts,
            ranking.input_tokens,
        );
        Ok(ranking.hits)
    }
}

#[cfg(test)]
#[path = "ranker_tests.rs"]
mod tests;
