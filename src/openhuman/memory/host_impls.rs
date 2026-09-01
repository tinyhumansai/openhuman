//! Host implementations of the seam traits `tinymemory-core` declares.
//!
//! The extracted memory subsystem reaches back into OpenHuman through nine
//! traits (see `tinymemory_api::host` and the `*_host` modules in
//! `tinymemory_core`). [`super::host`] carries the two that are about *data* —
//! `MemoryHostConfig` and `MemoryEventSink`. This module carries the seven that
//! are about *capability*: building providers, loading config, running spaCy,
//! throttling background work, reporting errors.
//!
//! # This whole module is behind `memory-engine-seams` (#5560)
//!
//! `tinymemory-core` has left the product build. It is a `[dev-dependencies]`
//! entry plus an `optional = true` normal one that only `memory-engine-seams`
//! and `rss-bench` turn on, so the engine these seams install into does not
//! exist in anything shipped — and neither does this module.
//!
//! The gate is a **feature** rather than `#[cfg(test)]`, and the reason is
//! worth knowing before "simplifying" it: a `tests/*.rs` integration target
//! links this crate as an ordinary dependency, where `cfg(test)` is false, so a
//! `#[cfg(test)]` module is invisible to it however the engine is declared. Two
//! dozen of those targets call [`install_memory_host_seams`], and several of
//! them drive a real in-process engine — the archivist, session-turn and
//! memory-sync cases in `raw_coverage_all` fail with "no EmbeddingHost
//! installed" without it, which is the same failure the first attempt at #5560
//! shipped to users. `memory-engine-seams` is default-ON, product-OFF and
//! allow-listed in `INTENTIONALLY_NOT_FORWARDED`.
//!
//! **The feature makes the engine available, not used.** In a default build
//! nothing in production reaches it: the boot sites install only the contract
//! event sink, so a linked-but-unwired engine is never called and the seams'
//! fail-loud behaviour is never triggered.
//!
//! Production reaches the engine through the loaded TinyMemory module over the
//! bus, and
//! `modules::memory_host` serves the same seven capabilities there through the
//! module's own inbound interfaces, which are a different mechanism entirely: a
//! `cdylib` has its own statics, so nothing installed here was ever visible to
//! it, and nothing it installs is visible here.
//!
//! **The contract event sink is not one of these seams and is not gated.** It
//! installs into `tinymemory_api`, which is still a normal dependency, and
//! `memory::sync::composio::bus` publishes `ComposioIntegrationsChanged`
//! through it from production host code. Each boot site calls
//! [`super::host::install_memory_event_sink`] directly for that reason —
//! `tinymemory_api::events::publish` drops silently when unwired, so folding it
//! in here would have removed a live event path without a single error.
//!
//! # They are process-globals, installed once
//!
//! Every one is reached through a `set_*` installer that
//! [`install_memory_host_seams`] calls, before any memory work begins. That
//! mirrors the shape the subsystem had before the extraction, when these were
//! free functions it called directly; [`install_for_tests`] is the one caller
//! that matters now.
//!
//! # Why several of them capture an `Arc<Config>`
//!
//! Four of the seams take a config on the seam side but delegate to a host
//! function whose signature does not (`resolve_api_key`, `ollama_base_url`,
//! `api_key`). Those impls hold the config the installer was given. It is the
//! startup config: a mid-session settings change is *not* reflected, which
//! matches how the pre-extraction call sites behaved — they read the same
//! ambient config — but is worth knowing before adding a seam method that
//! should be live. Seams that must be live (`ConfigLoader`) take a `&Config`
//! argument and re-read instead.
//!
//! # How this came to be test-only, and the wrong argument for it (#5560)
//!
//! `memory::binding` refuses `DriverClass::Embedded` outright — "embedded
//! memory drivers are no longer supported; use the 'tinymemory' module driver"
//! — and `modules::memory_host` serves this same set of seven over the bus for
//! the module. Both facts together read like an argument that the in-process
//! installs below are only reached from tests, and #5560 acted on exactly that
//! reading once and had to be reverted. **The driver class was the wrong thing
//! to check**: it governs what answers a `MemoryProvider` call and says nothing
//! about the free-function engine surface a call site can reach around the
//! binding entirely — the "second unpoliced door" `memory::direct_engine_refs`
//! is a ratchet over. The argument that eventually held is the one below:
//! caller by caller, until the free-function surface had no production reader
//! and the crate could leave the manifest.
//!
//! **Every production path this section used to name is now gone.** They went
//! one contract round at a time, and the list is worth keeping because it is
//! the shape the remaining work takes:
//!
//! - `agent::harness::archivist::recap` folds through `MemoryTree::summarise`.
//! - `memory::tools::doctor` runs through `MemoryMaintenance::diagnose`.
//! - `memory::tree::tree_runtime` — the last production glob
//!   (`pub use tinymemory_core::tree::tree_runtime::*`) — is deleted. Its five
//!   `tree_summarizer_*` RPC handlers, the `openhuman tree-summarizer` CLI,
//!   `memory::ops::learn` and the channels-startup subscriber ran the markdown
//!   time tree in *this* process and built its fold through
//!   `chat_host::create_chat_model_with_model_id`; they go over the bus now,
//!   through the six runtime-tree doors.
//! - `memory::tools::flavour` reached `tinycortex::memory::tree` directly and
//!   is on `MemoryTree::flavour_profile`.
//!
//! So **no production caller reaches an in-process engine fold any more**, and
//! [`ChatHost`] below is reached only from the far side of the bus, where
//! `modules::memory_host` serves it for the loaded module. That is not the same
//! thing as the installs being dead: unwire them and the module's own
//! summariser run fails with "no ChatHost installed", which is the failure
//! #5560 shipped once as "no EmbeddingHost installed" on the chat hot path. The
//! seams fail **loudly** rather than degrading, which is a property to keep.
//!
//! [`ChatHost::summarizer_available`] still delegates into
//! `tree_runtime::ops::summarizer_available`, and that is now the *only* edge
//! left between this file and that module: the host owns the local-AI /
//! cloud-opt-in precedence, and the seam is how the driver asks about it.
//!
//! **Everything left in this file is a seam install (#5560).** There used to be
//! one exception, `reset_in_process_chunk_store`, and the shape of its removal
//! is worth keeping because it is the shape the rest of this file's removal
//! takes. It dropped this process's cached SQLite handle after the *module*
//! quarantined and rebuilt `chunks.db`, because the host's own engine copy still
//! pointed at the renamed inode and every in-process read kept failing with
//! `database disk image is malformed` until restart (openhuman#5820). It was
//! originally justified by `memory::sources::status` reading that store over raw
//! SQLite, and later by "only this process can drop **this** process's handle" —
//! true, and beside the point once nothing in this process opens the store.
//!
//! That is now the case. `sources::status` asks
//! `MemoryChunks::source_ingest_status`; recall resolves through
//! `memory::binding` to the same module driver; and every surviving opener of
//! the host's chunk store is `#[cfg(test)]` — `read_rpc::with_connection`,
//! `tree::retrieval::test_support`, `security::credentials`'s ops tests and
//! `memory::sync_pipeline`'s. So the reset had no reader left to protect, and
//! `recover_corrupt_db` was itself the last production call that *opened* the
//! in-process chunk store. Deleting it removes a door rather than leaving one
//! ajar; the user-visible notice is untouched, because it was never the reset's
//! — `modules::memory_host`'s `into_domain_event` publishes it and returns
//! `None`, exactly as `memory::host`'s in-process sink does.
//!
//! **The question to re-ask was not "is the driver embedded" but "does any
//! production caller still reach an engine free function".** That inventory is
//! now empty, and it was emptied rather than argued away: `session::builder::
//! factory` stopped booting `global::init(workspace).memory_handle()`,
//! `ops::helpers::active_memory_client` was deleted, and `memory_cli`'s
//! `ingest`/`query` engine-client resolver went with it. What is left naming
//! `tinymemory_core::` is `#[cfg(test)]`, which the `[dev-dependencies]` entry
//! serves and the shipped binary does not link.
//!
//! **That grep is not the inventory for #5560 as a whole, and the difference
//! matters.** #5560 sheds two crates, and `memory::direct_engine_refs` ratchets
//! one needle. `tinycortex` is a direct dependency of this crate in its own
//! right — not something reached through `tinymemory-core` — so repointing a
//! file from `tinymemory_core::x` to `tinycortex::x` drops it out of that lint
//! while the engine stays linked. `memory::tree::health` did exactly that, on
//! the sound reasoning that the taxonomy was always `tinycortex`'s and the
//! engine crate only re-exported it. Add `tinycortex::` to the grep before
//! concluding the engine has left the build — at the time of writing the only
//! production file it still finds is `src/bin/library_profile/scenarios/
//! memory_ingest.rs`, which names both crates.
//!
//! # Composio no longer has a seam here
//!
//! `tinymemory_core::composio_host` was deleted in tinymemory v1.13.4 along
//! with the whole in-process Composio sync pipeline: reaching a connected
//! account needs a credential this crate must not hold. Composio sync is now
//! host-initiated — `memory::sync::composio` drives it through the
//! `tinyconnectors` module (see `modules::connectors`) and hands the resulting
//! records to the driver through `MemorySourceSink::accept_source_items`,
//! rather than the driver calling back into a host-installed seam.

use std::sync::Arc;

use async_trait::async_trait;
use tinymemory_api::host::{
    EmbeddingHost, EmbeddingProvider, ErrorReporter, Policy, SpacyResponse, UsageInfo,
};
use tinymemory_core::chat_host::ChatHost;
use tinymemory_core::config_loader::ConfigLoader;
use tinymemory_core::nlp_host::NlpHost;
use tinymemory_core::scheduler_gate::SchedulerGate;
use tinymemory_core::shutdown::{ShutdownHook, ShutdownHost};
use tokio::sync::Notify;

use crate::openhuman::config::Config;

/// Type alias for the seam's config trait object, to keep signatures readable.
///
/// Named on the contract crate rather than on `tinymemory_core::Config`, which
/// is nothing but `pub type Config = dyn tinymemory_api::host::
/// MemoryHostConfig;` — the same trait object under a longer chain. Spelling it
/// this way is not cosmetic: it means every remaining `tinymemory_core::` line
/// in this file is a *seam trait*, a *seam installation* or the in-process
/// recovery door, so the direct-reference inventory reads as what actually
/// keeps the engine linked here rather than as a mix of those and inert
/// aliases (#5560).
///
/// `SpacyResponse` and `Policy` were the two aliases that still broke that
/// rule, and they are imported from `tinymemory_api::host` above for the same
/// reason. It is the identical item either way — `tinymemory_core::nlp_host`
/// and `::scheduler_gate` are each a `pub use` of the contract's type — so the
/// repoint is free, and what it buys is that a reader counting engine
/// references in this file counts only things that would have to be *replaced*
/// rather than merely *renamed*. The traits themselves (`ChatHost`,
/// `ConfigLoader`, `NlpHost`, `SchedulerGate`, `ShutdownHost`) have no contract
/// declaration and cannot follow: they are the in-process embedding seam, which
/// is the half of `tinymemory_api::host` that never crosses the bus.
type SeamConfig = dyn tinymemory_api::host::MemoryHostConfig;

// ── Embeddings ──────────────────────────────────────────────────────────────

/// Builds embedding providers for the memory subsystem.
#[derive(Debug)]
pub struct OpenHumanEmbeddingHost {
    config: Arc<Config>,
}

impl EmbeddingHost for OpenHumanEmbeddingHost {
    fn resolve_api_key(&self, provider: &str) -> Option<String> {
        let key = crate::openhuman::inference::embeddings::resolve_api_key(&self.config, provider);
        // The host returns "" for "no credential stored"; the seam distinguishes
        // absence from an empty key so callers can report the difference.
        (!key.is_empty()).then_some(key)
    }

    fn ollama_base_url(&self) -> String {
        crate::openhuman::inference::local::ollama_base_url_from_config(&self.config)
    }

    fn default_embedding_provider(&self) -> Arc<dyn EmbeddingProvider> {
        // Scope the managed embedder to THIS host's config credential store, not
        // the keyless `default_state_dir()` hardcode. The memory client caches
        // this provider for the process lifetime, so a keyless scope that misses
        // the signed-in user's `app-session` token makes every ingested
        // document persist vector-less while "Test connection" (config-scoped)
        // still passes — #5501.
        crate::openhuman::inference::embeddings::default_embedding_provider_with_config(
            &self.config,
        )
    }

    fn create_embedding_provider_with_credentials(
        &self,
        provider: &str,
        model: &str,
        dims: usize,
        api_key: &str,
        custom_endpoint: Option<&str>,
    ) -> Result<Box<dyn EmbeddingProvider>, String> {
        crate::openhuman::inference::embeddings::create_embedding_provider_with_credentials(
            provider,
            model,
            dims,
            api_key,
            custom_endpoint,
        )
        .map_err(|e| format!("{e:#}"))
    }

    fn model_supports_dimensions(&self, model: &str) -> bool {
        crate::openhuman::inference::embeddings::model_supports_dimensions(model)
    }

    fn cloud_embedding_provider(
        &self,
        model: &str,
        dims: usize,
    ) -> Result<Box<dyn EmbeddingProvider>, String> {
        Ok(Box::new(
            crate::openhuman::inference::embeddings::cloud::OpenHumanCloudEmbedding::new(
                None,
                self.config
                    .config_path
                    .parent()
                    .map(std::path::PathBuf::from),
                self.config.secrets.encrypt,
                model,
                dims,
            ),
        ))
    }

    fn default_cloud_embedding_model(&self) -> &str {
        crate::openhuman::inference::embeddings::DEFAULT_CLOUD_EMBEDDING_MODEL
    }

    fn default_cloud_embedding_dimensions(&self) -> usize {
        crate::openhuman::inference::embeddings::DEFAULT_CLOUD_EMBEDDING_DIMENSIONS
    }

    fn ollama_embedding_provider(
        &self,
        base_url: &str,
        model: &str,
        dims: usize,
    ) -> Result<Box<dyn EmbeddingProvider>, String> {
        self.create_embedding_provider_with_credentials("ollama", model, dims, "", Some(base_url))
    }
}

// ── Chat models ─────────────────────────────────────────────────────────────

/// Builds chat models for summarisation and the memory chat helper.
///
/// Routing reads BYOK fallbacks, per-role routes and credentials, so it needs
/// the host's own `Config` — recovered from the trait object with
/// [`host_config`]. The captured config is only the fallback for a caller that
/// handed us somebody else's implementation.
#[derive(Debug)]
pub struct OpenHumanChatHost {
    config: Arc<Config>,
}

impl ChatHost for OpenHumanChatHost {
    fn provider_for_role(&self, role: &str, config: &SeamConfig) -> String {
        crate::openhuman::inference::provider::provider_for_role(
            role,
            host_config(config, &self.config),
        )
    }

    fn create_chat_model_with_model_id(
        &self,
        role: &str,
        config: &SeamConfig,
        temperature: f64,
    ) -> Result<(Arc<dyn tinyinference::model::ChatModel<()>>, String), String> {
        crate::openhuman::inference::provider::create_chat_model_with_model_id(
            role,
            host_config(config, &self.config),
            temperature,
        )
        .map_err(|e| format!("{e:#}"))
    }

    fn usage_from_response(
        &self,
        response: &tinyinference::model::ModelResponse,
    ) -> Option<UsageInfo> {
        crate::openhuman::agent::tinyagents::model::usage_info_from_response(response)
    }

    fn summarizer_available(&self, config: &SeamConfig) -> (bool, &'static str) {
        crate::openhuman::memory::tree::tree_runtime::ops::summarizer_available(host_config(
            config,
            &self.config,
        ))
    }
}

// ── Config loading ──────────────────────────────────────────────────────────

/// Loads host configs for the memory subsystem's background loops.
#[derive(Debug)]
pub struct OpenHumanConfigLoader;

#[async_trait]
impl ConfigLoader for OpenHumanConfigLoader {
    async fn load(&self) -> Result<Box<SeamConfig>, String> {
        Ok(Box::new(
            crate::openhuman::config::rpc::load_config_with_timeout().await?,
        ))
    }

    async fn reload_snapshot(&self, snapshot: &SeamConfig) -> Result<Arc<SeamConfig>, String> {
        // Addressed by path, not by the whole config: the caller holds the
        // seam's trait object and cannot hand us a concrete `Config`.
        let config = crate::openhuman::config::rpc::reload_config_from_paths(
            snapshot.config_path(),
            snapshot.workspace_dir(),
        )
        .await?;
        Ok(Arc::new(config))
    }
}

// ── spaCy ───────────────────────────────────────────────────────────────────

/// Runs spaCy extraction through the host's Python runtime.
#[derive(Debug)]
pub struct OpenHumanNlpHost;

#[async_trait]
impl NlpHost for OpenHumanNlpHost {
    async fn extract_spacy(
        &self,
        config: &SeamConfig,
        text: &str,
    ) -> Result<SpacyResponse, String> {
        let config = live_config(config).await?;
        crate::openhuman::runtime::python_server::extract_spacy(&config, text)
            .await
            .map_err(|e| format!("{e:#}"))
    }
}

// ── Scheduler gate ──────────────────────────────────────────────────────────

/// Exposes the host's background-AI throttle.
#[derive(Debug)]
pub struct OpenHumanSchedulerGate;

#[async_trait]
impl SchedulerGate for OpenHumanSchedulerGate {
    fn current_policy(&self) -> Policy {
        crate::openhuman::cron::scheduler_gate::gate::current_policy()
    }

    fn resume_notify(&self) -> Arc<Notify> {
        crate::openhuman::cron::scheduler_gate::gate::resume_notify()
    }

    async fn wait_for_capacity(&self) -> Option<Box<dyn Send>> {
        crate::openhuman::cron::scheduler_gate::wait_for_capacity()
            .await
            .map(|permit| Box::new(permit) as Box<dyn Send>)
    }
}

// ── Shutdown ────────────────────────────────────────────────────────────────

/// Registers memory shutdown hooks with the host's shutdown sequencer.
#[derive(Debug)]
pub struct OpenHumanShutdownHost;

impl ShutdownHost for OpenHumanShutdownHost {
    fn register(&self, hook: ShutdownHook) {
        let hook = Arc::new(hook);
        crate::core::shutdown::register(move || {
            let hook = Arc::clone(&hook);
            async move { hook().await }
        });
    }
}

// ── Error reporting ─────────────────────────────────────────────────────────

/// Routes memory error reports into the host's observability pipeline.
#[derive(Debug)]
pub struct OpenHumanErrorReporter;

impl ErrorReporter for OpenHumanErrorReporter {
    fn report_error(&self, rendered: &str, domain: &str, operation: &str, tags: &[(&str, &str)]) {
        crate::core::observability::report_error(rendered, domain, operation, tags);
    }

    fn report_error_or_expected(
        &self,
        rendered: &str,
        domain: &str,
        operation: &str,
        tags: &[(&str, &str)],
    ) {
        crate::core::observability::report_error_or_expected(rendered, domain, operation, tags);
    }
}

// ── Wiring ──────────────────────────────────────────────────────────────────

/// Recover the host's concrete `Config` from the seam's trait object.
///
/// Returns `fallback` when the config is some other implementor — a test
/// double, say. That is not an error: it means there is no host config to
/// recover, and the one the seam was installed with is the best answer.
fn host_config<'a>(config: &'a SeamConfig, fallback: &'a Config) -> &'a Config {
    config.as_any().downcast_ref::<Config>().unwrap_or(fallback)
}

/// Re-read the host's concrete `Config` from the paths the seam points at.
///
/// The seam is `dyn MemoryHostConfig` and the host functions these impls
/// delegate to want `&Config`. Recovering one is a file read, so it is only
/// available to the **async** seam methods; the sync ones use the `Arc<Config>`
/// captured at install time instead, and say so on their impl.
///
/// Deliberately not a downcast: `TestHostConfig` and any future implementor are
/// not the host's `Config`, and a downcast would turn them into a silent `None`
/// rather than an honest re-read.
///
/// # Errors
///
/// Returns `Err` when the config file cannot be read.
async fn live_config(config: &SeamConfig) -> Result<Config, String> {
    crate::openhuman::config::rpc::reload_config_from_paths(
        config.config_path(),
        config.workspace_dir(),
    )
    .await
}

/// Install every host seam into `tinymemory-core`.
///
/// Call once during startup wiring, **before any memory work begins** — the
/// embedding, chat and config seams all fail loudly when unwired, by design,
/// because degrading quietly would corrupt an embedding space or make a sync
/// run look empty rather than broken. Composio has no seam here any more —
/// see the module docs.
pub fn install_memory_host_seams(config: Arc<Config>) {
    tinymemory_core::embedding_host::set_embedding_host(Arc::new(OpenHumanEmbeddingHost {
        config: Arc::clone(&config),
    }));
    tinymemory_core::chat_host::set_chat_host(Arc::new(OpenHumanChatHost {
        config: Arc::clone(&config),
    }));
    tinymemory_core::config_loader::set_config_loader(Arc::new(OpenHumanConfigLoader));
    tinymemory_core::nlp_host::set_nlp_host(Arc::new(OpenHumanNlpHost));
    tinymemory_core::scheduler_gate::set_scheduler_gate(Arc::new(OpenHumanSchedulerGate));
    tinymemory_core::shutdown::set_shutdown_host(Arc::new(OpenHumanShutdownHost));
    tinymemory_core::observability::set_error_reporter(Arc::new(OpenHumanErrorReporter));
    super::host::install_memory_event_sink();
    log::debug!("[memory:host] all seam implementations installed");
}

/// Install the seams for this crate's own tests.
///
/// Before the extraction, memory code called `inference::embeddings` and the
/// provider factory directly, so any test that built a memory client got the
/// real implementations for free. The seams made that wiring explicit — which
/// is the point at runtime, but it means a test that builds a client now has to
/// say so. This installs exactly what used to be implicit: the real host impls,
/// over a default config.
///
/// Idempotent, and safe to call from many test threads.
///
/// # Why the thread
///
/// `Config` is a large struct, and `Config::default()` materialises one on the
/// caller's stack before it reaches the `Arc`. Most callers here are
/// `#[tokio::test]` async fns whose futures are already deep; adding it inline
/// overflows the 2 MiB test-thread stack. Building it on a thread with a stack
/// of its own keeps the cost off the caller entirely, and `Once` means it
/// happens exactly one time per test binary.
#[cfg(test)]
pub(crate) fn install_for_tests() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        std::thread::Builder::new()
            .name("memory-seam-install".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| install_memory_host_seams(Arc::new(Config::default())))
            .expect("spawn seam installer")
            .join()
            .expect("seam installer panicked");
    });
}

#[cfg(test)]
#[path = "host_impls_boot_seam_tests_tests.rs"]
mod boot_seam_tests;
