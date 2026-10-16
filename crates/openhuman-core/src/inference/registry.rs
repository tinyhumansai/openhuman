//! Registered inference providers, keyed by provider id.
//!
//! Three tables an outside crate fills to supply a provider the core does not
//! build itself, each consulted **before** the built-in construction:
//!
//! | Table | Consulted by | Registered value |
//! | --- | --- | --- |
//! | chat models | `provider::factory` (every managed-provider construction) | [`ChatModelEntry`] |
//! | embeddings | `embedding_host::factory` (the managed embedding model) | [`EmbeddingFactory`] |
//! | model catalogs | `provider::ops::models::catalog_listing`, `provider::factory::discovery` | [`CatalogEntry`] |
//!
//! The id is the provider id the core already routes on (the managed provider
//! is [`PROVIDER_OPENHUMAN`](crate::inference::provider::factory::PROVIDER_OPENHUMAN)).
//! The tables know nothing about what a provider talks to. With nothing
//! registered every consult point behaves exactly as before.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tinyinference_embeddings::EmbeddingModel;
use tinyinference_llm::model::discover::DiscoveryRequest;
use tinyinference_llm::model::ChatModel;

use crate::config::Config;
use crate::core::slots::{Extensions, HookSlot};
use crate::core::Outcome;
use crate::security::approval::ApprovalChatContext;
use crate::security::egress::EgressLedger;

/// A boxed `Send` future.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

// ---------------------------------------------------------------------------
// Chat models
// ---------------------------------------------------------------------------

/// What a registered chat-model factory is asked to build.
pub struct ChatModelRequest<'a> {
    /// The workload role the route was resolved for (`chat`, `reasoning`, ...).
    pub role: &'a str,
    /// The configuration in force.
    pub config: &'a Config,
    /// The concrete model id the core resolved for this construction (a role
    /// alias such as `hint:coding` has already been translated to the
    /// configured default).
    pub model_override: Option<&'a str>,
    /// The model the turn runs on when the request itself names none.
    pub default_model: Option<&'a str>,
    /// Whether the model should use native tool calling.
    pub native_tool_calling: bool,
    /// The conversation thread this model serves, when it is a turn model.
    pub thread_id: Option<&'a str>,
    /// Where the construction discloses its external transfer.
    pub egress: &'a EgressLedger,
}

/// Builds a chat model and reports the model id it resolved to.
pub type ChatModelFactory = Arc<
    dyn for<'a> Fn(&ChatModelRequest<'a>) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)>
        + Send
        + Sync,
>;

/// Asks a provider whether it can serve `role` right now: `(role, model,
/// config)`, where `model` is the concrete model id the core resolved.
pub type ReadinessProbe =
    Arc<dyn Fn(String, String, Config) -> BoxFuture<Result<(), String>> + Send + Sync>;

/// A registered chat-model provider.
#[derive(Clone)]
pub struct ChatModelEntry {
    /// Builds the model.
    pub build: ChatModelFactory,
    /// Optional readiness probe (the flows inference probe).
    pub probe: Option<ReadinessProbe>,
}

static CHAT_MODELS: HookSlot<ChatModelEntry> = HookSlot::new("inference::chat_models");

/// Registers the chat-model provider `provider_id` for the whole process.
pub fn register_chat_model(provider_id: impl Into<String>, entry: ChatModelEntry) {
    CHAT_MODELS.register(provider_id, entry);
}

/// Registers the chat-model provider `provider_id` for the context built from
/// `extensions`.
pub fn register_chat_model_in(
    extensions: &Extensions,
    provider_id: impl Into<String>,
    entry: ChatModelEntry,
) {
    CHAT_MODELS.register_in(extensions, provider_id, entry);
}

/// Removes the process-wide chat-model provider `provider_id`.
pub fn unregister_chat_model(provider_id: &str) -> bool {
    CHAT_MODELS.unregister(provider_id)
}

/// The registered chat-model provider, if any.
pub(crate) fn chat_model(provider_id: &str) -> Option<ChatModelEntry> {
    let registered = CHAT_MODELS.get(provider_id);
    // Unit tests model "a hosted provider is present" by default, as the old
    // built-in managed backend was; a test that wants the bare core asks for it
    // with `without_managed_provider`.
    #[cfg(test)]
    let registered =
        registered.or_else(|| super::registry_test_support::default_managed_entry(provider_id));
    registered
}

// ---------------------------------------------------------------------------
// Embeddings
// ---------------------------------------------------------------------------

/// What a registered embedding factory is asked to build.
pub struct EmbeddingRequest<'a> {
    /// The configuration in force, when the caller has one (the credential-only
    /// constructors do not).
    pub config: Option<&'a Config>,
    /// The embedding model id.
    pub model: &'a str,
    /// The vector dimensionality.
    pub dimensions: usize,
    /// The chat turn the call is made in, so disclosures reach that chat.
    pub chat: Option<ApprovalChatContext>,
    /// The directory the caller's credentials live in (the config's credential
    /// scope, or the process default when the caller has no config).
    pub state_dir: std::path::PathBuf,
    /// Whether that credential store is encrypted at rest.
    pub secrets_encrypt: bool,
}

/// Builds an embedding model.
pub type EmbeddingFactory = Arc<
    dyn for<'a> Fn(
            &EmbeddingRequest<'a>,
        ) -> tinyinference_embeddings::Result<Box<dyn EmbeddingModel>>
        + Send
        + Sync,
>;

static EMBEDDINGS: HookSlot<EmbeddingFactory> = HookSlot::new("inference::embeddings");

/// Registers the embedding provider `provider_id` for the whole process.
pub fn register_embedding<F>(provider_id: impl Into<String>, factory: F)
where
    F: for<'a> Fn(
            &EmbeddingRequest<'a>,
        ) -> tinyinference_embeddings::Result<Box<dyn EmbeddingModel>>
        + Send
        + Sync
        + 'static,
{
    EMBEDDINGS.register(provider_id, Arc::new(factory));
}

/// Registers the embedding provider `provider_id` for the context built from
/// `extensions`.
pub fn register_embedding_in<F>(extensions: &Extensions, provider_id: impl Into<String>, factory: F)
where
    F: for<'a> Fn(
            &EmbeddingRequest<'a>,
        ) -> tinyinference_embeddings::Result<Box<dyn EmbeddingModel>>
        + Send
        + Sync
        + 'static,
{
    EMBEDDINGS.register_in(extensions, provider_id, Arc::new(factory));
}

/// Removes the process-wide embedding provider `provider_id`.
pub fn unregister_embedding(provider_id: &str) -> bool {
    EMBEDDINGS.unregister(provider_id)
}

/// The registered embedding factory, if any.
pub(crate) fn embedding(provider_id: &str) -> Option<EmbeddingFactory> {
    EMBEDDINGS.get(provider_id)
}

// ---------------------------------------------------------------------------
// Model catalogs
// ---------------------------------------------------------------------------

/// What the core resolved for the `cloud_providers` entry a listing serves,
/// handed to the catalog so it honours the same per-entry settings every
/// other provider listing does.
#[derive(Clone)]
pub struct CatalogRequest {
    /// The key stored for the entry's slug (empty when none): the credential
    /// a catalog falls back to when there is no live session.
    pub provider_key: String,
    /// The HTTP client for the entry: the entry's CA bundle when it has one,
    /// else the runtime proxy client, under the `providers.list_models`
    /// service key.
    pub client: reqwest::Client,
}

/// Lists a provider's models: the same envelope `list_configured_models`
/// returns (`{ "models": [...] }`).
pub type CatalogListing = Arc<
    dyn Fn(Config, CatalogRequest) -> BoxFuture<Result<Outcome<serde_json::Value>, String>>
        + Send
        + Sync,
>;

/// Where to ask for one model's limits: `(model, config)`; `None` when the
/// provider has no listing for it.
pub type CatalogLimits = Arc<dyn Fn(&str, &Config) -> Option<DiscoveryRequest> + Send + Sync>;

/// A registered model catalog.
#[derive(Clone)]
pub struct CatalogEntry {
    /// The `list_configured_models` listing.
    pub list: CatalogListing,
    /// The limits-discovery request, when the provider has one.
    pub limits: Option<CatalogLimits>,
}

static CATALOGS: HookSlot<CatalogEntry> = HookSlot::new("inference::catalogs");

/// Registers the model catalog for `provider_id` for the whole process.
pub fn register_catalog(provider_id: impl Into<String>, entry: CatalogEntry) {
    CATALOGS.register(provider_id, entry);
}

/// Registers the model catalog for `provider_id` for the context built from
/// `extensions`.
pub fn register_catalog_in(
    extensions: &Extensions,
    provider_id: impl Into<String>,
    entry: CatalogEntry,
) {
    CATALOGS.register_in(extensions, provider_id, entry);
}

/// Removes the process-wide model catalog for `provider_id`.
pub fn unregister_catalog(provider_id: &str) -> bool {
    CATALOGS.unregister(provider_id)
}

/// The registered catalog, if any.
pub(crate) fn catalog(provider_id: &str) -> Option<CatalogEntry> {
    CATALOGS.get(provider_id)
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
