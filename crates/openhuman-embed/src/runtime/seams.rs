//! Process-global core seams as builder options.
//!
//! The core exposes a handful of installers a host calls once per process:
//! the controller registry extension, the `tool_search` ranker, embedder
//! post-turn and tool hooks, the `run`/`serve` server launcher and the live
//! security policy. [`RuntimeBuilder`] takes each as an option, installs it on
//! `build()`, and restores what it can when the runtime is dropped:
//!
//! | Seam | Installed | On drop |
//! |---|---|---|
//! | [`controller_extension`](RuntimeBuilder::controller_extension) | before boot | kept: the registry has no removal; re-registering the same controllers is a no-op |
//! | [`tool_ranker`](RuntimeBuilder::tool_ranker) | before boot | the previous ranker is restored, if ours still holds the slot |
//! | [`post_turn_hook`](RuntimeBuilder::post_turn_hook) / [`tool_hook`](RuntimeBuilder::tool_hook) | before boot; learning sinks get private runtime identities, tool hooks retain declared names | newest first, while ours still holds the identity: ours removed, a replaced hook restored |
//! | [`server_launcher`](RuntimeBuilder::server_launcher) | before boot | kept: first install wins for the process |
//! | [`live_policy`](RuntimeBuilder::live_policy) | after boot | kept: the next boot (or a config reload) installs its own |
//! | [`storage`](RuntimeBuilder::storage) | before boot | the previous backend is restored (or cleared), if ours still holds the slot |
//!
//! The live policy goes in *after* boot because the core's bootstrap installs
//! one derived from the config; installing earlier would be overwritten. A
//! later `config.update_autonomy_settings` reload or channel startup rebuilds
//! the policy from config again — the override lasts until then.

use std::sync::Arc;

use crate::seams::{LearningSink, OpenHumanRunContext, ToolHook};
#[cfg(test)]
use openhuman_core::agent::hooks::embedder_post_turn_hooks;
use openhuman_core::agent::hooks::{
    embedder_post_turn_hook_named, embedder_tool_hooks, replace_embedder_post_turn_hook,
    replace_embedder_tool_hook,
};
use openhuman_core::agent::tinyagents::discovery::{
    clear_tool_ranker, install_tool_ranker, installed_tool_ranker,
};
use openhuman_core::core::all::{register_controller_extension, ControllerExtension};
use openhuman_core::core::server_launcher::{install_server_launcher, ServerLauncher};
use openhuman_core::core::slots::Extensions;
use openhuman_core::security::SecurityPolicy;
use openhuman_core::storage::StorageBackend;
use tinytools::ToolRanker;

use super::RuntimeBuilder;

/// Where a runtime's storage backend (the `tinystoragedrivers` ports the
/// core's `storage` domain reads) comes from.
#[derive(Clone)]
pub enum StorageSource {
    /// A storage URL (`sqlite://…`, `memory`, `mongodb://…`), opened when the
    /// runtime is built. A URL that cannot be opened fails the build.
    Url(String),
    /// A backend the host already opened.
    Backend(Arc<dyn StorageBackend>),
}

impl std::fmt::Debug for StorageSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A URL can carry credentials: never print it.
        match self {
            Self::Url(_) => f.write_str("StorageSource::Url(<redacted>)"),
            Self::Backend(backend) => write!(f, "StorageSource::Backend({})", backend.driver()),
        }
    }
}

impl From<&str> for StorageSource {
    fn from(url: &str) -> Self {
        Self::Url(url.to_string())
    }
}

impl From<String> for StorageSource {
    fn from(url: String) -> Self {
        Self::Url(url)
    }
}

impl From<Arc<dyn StorageBackend>> for StorageSource {
    fn from(backend: Arc<dyn StorageBackend>) -> Self {
        Self::Backend(backend)
    }
}

/// The seam options a builder collected, not yet installed.
#[derive(Default)]
pub(crate) struct HostSeams {
    pub(crate) controller_extensions: Vec<ControllerExtension>,
    pub(crate) core_extensions: Extensions,
    pub(crate) tool_ranker: Option<Arc<dyn ToolRanker>>,
    pub(crate) post_turn_hooks: Vec<Arc<dyn LearningSink>>,
    pub(crate) tool_hooks: Vec<Arc<dyn ToolHook<OpenHumanRunContext>>>,
    pub(crate) server_launcher: Option<ServerLauncher>,
    pub(crate) live_policy: Option<Arc<SecurityPolicy>>,
    pub(crate) storage: Option<StorageSource>,
}

impl HostSeams {
    /// Open a [`StorageSource::Url`] into a backend, so [`install`](Self::install)
    /// can stay synchronous. A no-op for any other source.
    ///
    /// # Errors
    ///
    /// The URL could not be parsed or opened.
    pub(crate) async fn open_storage(&mut self) -> Result<(), String> {
        if let Some(StorageSource::Url(url)) = &self.storage {
            let backend = openhuman_core::storage::open(url)
                .await
                .map_err(|error| format!("opening the storage backend: {error}"))?;
            self.storage = Some(StorageSource::Backend(backend));
        }
        Ok(())
    }

    /// [`open_storage`](Self::open_storage) from synchronous code, on the
    /// core's dedicated storage runtime thread so the backend outlives the call.
    pub(crate) fn open_storage_blocking(&mut self) -> Result<(), String> {
        let Some(StorageSource::Url(url)) = self.storage.take() else {
            return Ok(());
        };
        let backend =
            openhuman_core::storage::block_on(
                async move { openhuman_core::storage::open(&url).await },
            )
            .map_err(|error| format!("opening the storage backend: {error}"))?;
        self.storage = Some(StorageSource::Backend(backend));
        Ok(())
    }

    /// Install every pre-boot seam and return the guard that undoes the
    /// restorable ones.
    ///
    /// Controller extensions go first because they are the only fallible
    /// step; nothing restorable has been touched when one is refused. An
    /// extension accepted before a later one is refused stays registered —
    /// the registry has no removal.
    pub(crate) fn install(self) -> Result<InstalledSeams, String> {
        for ext in self.controller_extensions {
            let group = ext.group;
            register_controller_extension(ext).map_err(|error| {
                log::warn!("[embed][seams] controller extension refused group={group:?}: {error}");
                error
            })?;
            log::debug!("[embed][seams] controller extension registered group={group:?}");
        }

        if let Some(launcher) = self.server_launcher {
            install_server_launcher(launcher);
            log::debug!("[embed][seams] server launcher installed (process lifetime)");
        }

        let ranker = self.tool_ranker.map(|ranker| {
            let previous = installed_tool_ranker();
            log::debug!(
                "[embed][seams] tool ranker installed kind={} replaced={}",
                ranker.kind(),
                previous.is_some()
            );
            install_tool_ranker(Arc::clone(&ranker));
            (ranker, previous)
        });

        let post_turn_hooks = self
            .post_turn_hooks
            .into_iter()
            .map(|hook| {
                static NEXT_LEARNING_SINK: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);
                let name = format!(
                    "embedder.learning_sink.{:020}",
                    NEXT_LEARNING_SINK.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
                let previous = embedder_post_turn_hook_named(&name);
                replace_embedder_post_turn_hook(&name, Some(Arc::clone(&hook)));
                log::debug!(
                    "[embed][seams] post-turn hook installed name={name} replaced={}",
                    previous.is_some()
                );
                InstalledHook {
                    name,
                    ours: hook,
                    previous,
                }
            })
            .collect();

        let tool_hooks = self
            .tool_hooks
            .into_iter()
            .map(|hook| {
                let name = hook.name().to_string();
                let previous = embedder_tool_hooks()
                    .into_iter()
                    .find(|existing| existing.name() == name);
                replace_embedder_tool_hook(&name, Some(Arc::clone(&hook)));
                log::debug!(
                    "[embed][seams] tool hook installed name={name} replaced={}",
                    previous.is_some()
                );
                InstalledHook {
                    name,
                    ours: hook,
                    previous,
                }
            })
            .collect();

        let storage = match self.storage {
            Some(StorageSource::Backend(backend)) => {
                let previous = openhuman_core::storage::install(Arc::clone(&backend));
                log::debug!(
                    "[embed][seams] storage backend installed driver={} replaced={}",
                    backend.driver(),
                    previous.is_some()
                );
                Some((backend, previous))
            }
            Some(StorageSource::Url(_)) => {
                return Err("storage url was not opened before install".to_string());
            }
            None => None,
        };

        Ok(InstalledSeams {
            storage,
            ranker,
            post_turn_hooks,
            tool_hooks,
            live_policy: self.live_policy,
            restore: true,
        })
    }
}

/// A ranker we installed, and what held the slot before it.
type InstalledRanker = (Arc<dyn ToolRanker>, Option<Arc<dyn ToolRanker>>);

/// A storage backend we installed, and what held the slot before it.
type InstalledStorage = (Arc<dyn StorageBackend>, Option<Arc<dyn StorageBackend>>);

/// A hook we installed under `name`, and what held that name before it.
struct InstalledHook<H: ?Sized> {
    name: String,
    ours: Arc<H>,
    previous: Option<Arc<H>>,
}

/// Seams a runtime installed. Dropping it restores the restorable ones; see
/// the module docs.
pub(crate) struct InstalledSeams {
    storage: Option<InstalledStorage>,
    ranker: Option<InstalledRanker>,
    /// Our hooks, in install order, each with the same-named hook it replaced.
    post_turn_hooks: Vec<InstalledHook<dyn LearningSink>>,
    tool_hooks: Vec<InstalledHook<dyn ToolHook<OpenHumanRunContext>>>,
    /// Installed only once the core has booted.
    live_policy: Option<Arc<SecurityPolicy>>,
    restore: bool,
}

impl InstalledSeams {
    pub(crate) fn post_turn_hook(&mut self, name: &str, hook: Option<Arc<dyn LearningSink>>) {
        while let Some(index) = self
            .post_turn_hooks
            .iter()
            .rposition(|installed| installed.name == name)
        {
            let installed = self.post_turn_hooks.remove(index);
            let current = embedder_post_turn_hook_named(name);
            if current.is_some_and(|current| Arc::ptr_eq(&current, &installed.ours)) {
                replace_embedder_post_turn_hook(name, installed.previous);
            }
        }
        if let Some(hook) = hook {
            let previous = embedder_post_turn_hook_named(name);
            replace_embedder_post_turn_hook(name, Some(hook.clone()));
            self.post_turn_hooks.push(InstalledHook {
                name: name.to_owned(),
                ours: hook,
                previous,
            });
        }
    }
    pub(crate) fn tool_hook(
        &mut self,
        name: &str,
        hook: Option<Arc<dyn ToolHook<OpenHumanRunContext>>>,
    ) {
        while let Some(index) = self
            .tool_hooks
            .iter()
            .rposition(|installed| installed.name == name)
        {
            let installed = self.tool_hooks.remove(index);
            let current = embedder_tool_hooks()
                .into_iter()
                .find(|hook| hook.name() == name);
            if current.is_some_and(|current| Arc::ptr_eq(&current, &installed.ours)) {
                replace_embedder_tool_hook(name, installed.previous);
            }
        }
        if let Some(hook) = hook {
            let hook: Arc<dyn ToolHook<OpenHumanRunContext>> =
                Arc::new(super::live_hooks::NamedToolHook(name.to_owned(), hook));
            let previous = embedder_tool_hooks()
                .into_iter()
                .find(|hook| hook.name() == name);
            replace_embedder_tool_hook(name, Some(hook.clone()));
            self.tool_hooks.push(InstalledHook {
                name: name.to_owned(),
                ours: hook,
                previous,
            });
        }
    }
    /// Install the live policy, if one was given, against the booted config's
    /// directories. Call after the core's bootstrap has installed its own.
    pub(crate) fn install_live_policy(
        &mut self,
        workspace_dir: &std::path::Path,
        action_dir: &std::path::Path,
    ) {
        if let Some(policy) = self.live_policy.take() {
            openhuman_core::security::live_policy::install(
                policy,
                workspace_dir.to_path_buf(),
                action_dir.to_path_buf(),
            );
            log::debug!("[embed][seams] live policy installed (until the next reload)");
        }
    }

    /// Whether a live policy is still waiting for the core to boot.
    pub(crate) fn has_pending_live_policy(&self) -> bool {
        self.live_policy.is_some()
    }

    /// Keep everything installed for the rest of the process — for an entry
    /// point that never returns control to a runtime ([`RuntimeBuilder::run_from_args`]).
    pub(crate) fn persist(mut self) {
        self.restore = false;
        log::debug!("[embed][seams] seams kept for the process lifetime");
    }
}

impl Drop for InstalledSeams {
    fn drop(&mut self) {
        if !self.restore {
            return;
        }
        if let Some((ours, previous)) = self.storage.take() {
            let still_ours =
                openhuman_core::storage::installed().is_some_and(|now| Arc::ptr_eq(&now, &ours));
            if still_ours {
                match previous {
                    Some(previous) => {
                        openhuman_core::storage::install(previous);
                    }
                    None => {
                        openhuman_core::storage::clear();
                    }
                }
                log::debug!("[embed][seams] storage backend restored");
            } else {
                log::debug!("[embed][seams] storage backend replaced since build; left alone");
            }
        }
        if let Some((ours, previous)) = self.ranker.take() {
            let still_ours = installed_tool_ranker().is_some_and(|now| Arc::ptr_eq(&now, &ours));
            if still_ours {
                match previous {
                    Some(previous) => install_tool_ranker(previous),
                    None => clear_tool_ranker(),
                }
                log::debug!("[embed][seams] tool ranker restored");
            } else {
                log::debug!("[embed][seams] tool ranker replaced since build; left alone");
            }
        }
        // Newest first: a second same-named hook recorded the first as its
        // `previous`, so unwinding in install order would put the first back
        // over the original. And only while ours still holds the name, so a
        // hook installed after build is not clobbered.
        while let Some(hook) = self.post_turn_hooks.pop() {
            let current = embedder_post_turn_hook_named(&hook.name);
            if current.is_some_and(|now| Arc::ptr_eq(&now, &hook.ours)) {
                replace_embedder_post_turn_hook(&hook.name, hook.previous);
                log::debug!("[embed][seams] post-turn hook restored name={}", hook.name);
            } else {
                log::debug!(
                    "[embed][seams] post-turn hook replaced since build; left alone name={}",
                    hook.name
                );
            }
        }
        while let Some(hook) = self.tool_hooks.pop() {
            let current = embedder_tool_hooks()
                .into_iter()
                .find(|existing| existing.name() == hook.name);
            if current.is_some_and(|now| Arc::ptr_eq(&now, &hook.ours)) {
                replace_embedder_tool_hook(&hook.name, hook.previous);
                log::debug!("[embed][seams] tool hook restored name={}", hook.name);
            } else {
                log::debug!(
                    "[embed][seams] tool hook replaced since build; left alone name={}",
                    hook.name
                );
            }
        }
    }
}

impl RuntimeBuilder {
    /// Configure context-scoped core hooks before this runtime is built.
    ///
    /// The callback receives the same extension set that is attached to the
    /// runtime's `CoreContext`; registrations remain isolated from other core
    /// contexts in the process. This is intended for layered host crates.
    #[doc(hidden)]
    pub fn configure_core_extensions(self, configure: impl FnOnce(&Extensions)) -> Self {
        configure(&self.seams.core_extensions);
        self
    }

    /// Contribute controllers to the core's registry, as the TinyHumans layer
    /// does for its hosted RPC proxies. May be called more than once.
    ///
    /// Registered before boot; a collision with a built-in or earlier
    /// extension fails the build with [`RuntimeError::Invalid`](super::RuntimeError::Invalid).
    /// The registry has no removal, so an extension outlives the runtime;
    /// registering identical controllers again is a no-op.
    pub fn controller_extension(mut self, extension: ControllerExtension) -> Self {
        self.seams.controller_extensions.push(extension);
        self
    }

    /// The process-wide `tool_search` ranker. Restored to the previous one
    /// when the runtime drops.
    pub fn tool_ranker(mut self, ranker: Arc<dyn ToolRanker>) -> Self {
        self.seams.tool_ranker = Some(ranker);
        self
    }

    /// A hook every agent session runs after each turn. The runtime registers
    /// it with a private identity before boot and removes it when the runtime
    /// drops.
    pub fn post_turn_hook(mut self, hook: Arc<dyn LearningSink>) -> Self {
        self.seams.post_turn_hooks.push(hook);
        self
    }

    /// A hook every agent session runs around each tool call. Same
    /// replacement and restore rules as [`post_turn_hook`](Self::post_turn_hook).
    pub fn tool_hook(mut self, hook: Arc<dyn ToolHook<OpenHumanRunContext>>) -> Self {
        self.seams.tool_hooks.push(hook);
        self
    }

    /// The launcher the CLI's `run` / `serve` subcommands start a server
    /// with. The first launcher installed in a process wins and stays.
    pub fn server_launcher(mut self, launcher: ServerLauncher) -> Self {
        self.seams.server_launcher = Some(launcher);
        self
    }

    /// The storage backend the core's `storage` domain reads (approvals,
    /// secrets, documents, per-agent scopes): a URL opened at build time, or a
    /// backend the host opened. Installed before boot and, when the runtime
    /// drops, the previous backend is restored (or cleared) if ours still holds
    /// the slot.
    ///
    /// This is only the backend. Conversations move onto it with
    /// [`session_store`](Self::session_store).
    pub fn storage(mut self, source: impl Into<StorageSource>) -> Self {
        self.seams.storage = Some(source.into());
        self
    }

    /// Replace the live security policy the core installs at boot. Applied
    /// after boot; a later autonomy-settings reload rebuilds it from config.
    pub fn live_policy(mut self, policy: Arc<SecurityPolicy>) -> Self {
        self.seams.live_policy = Some(policy);
        self
    }
}

#[cfg(test)]
#[path = "seams_tests.rs"]
mod tests;
