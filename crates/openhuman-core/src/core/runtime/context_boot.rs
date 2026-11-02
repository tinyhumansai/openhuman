use super::*;
use crate::core::runtime::TokenSource;

impl CoreContext {
    /// Run the core initialization sequence and return the context plus whether
    /// an operator-supplied RPC bearer exists (for the public-bind safety check
    /// in `openhuman_rpc::server::serve`) plus the loaded config, when boot reached
    /// workspace-bound init. Order is load-bearing and mirrors the original
    /// `run_server_inner` sequence:
    ///
    /// 1. register controllers, 2. master key, 3. seed RPC bearer,
    /// 4. workspace stores ([`init_stores`]), 5. pure runtime registration.
    ///
    /// `preloaded_config` lets an embedder supply the [`Config`] outright
    /// instead of having step 4 discover one from disk and the environment. See
    /// [`init_with_config`](Self::init_with_config) for why that matters.
    pub async fn init(
        host_kind: HostKind,
        token: &TokenSource,
        domains: crate::core::runtime::DomainSet,
    ) -> anyhow::Result<(Arc<CoreContext>, bool, Option<crate::config::Config>)> {
        Self::init_with_config(host_kind, token, domains, Default::default(), None).await
    }

    /// [`init`](Self::init) with an optional caller-supplied configuration.
    ///
    /// Passing `Some(config)` skips `Config::load_or_init()` entirely — the
    /// config is used verbatim, exactly as loaded config would be. This is the
    /// seam that lets a library embedder configure the core with struct fields
    /// rather than by mutating the process environment before `build()`, which
    /// is order-dependent, process-global, and invisible at the call site.
    ///
    /// Note it does not make the core hermetic on its own: `init_stores`, the
    /// session database and the keyring still write beneath
    /// `config.workspace_dir`. It decides *where*, not *whether*.
    pub async fn init_with_config(
        host_kind: HostKind,
        token: &TokenSource,
        domains: crate::core::runtime::DomainSet,
        tool_groups: crate::tools::toolpacks::ToolGroups,
        preloaded_config: Option<crate::config::Config>,
    ) -> anyhow::Result<(Arc<CoreContext>, bool, Option<crate::config::Config>)> {
        Self::init_with_boot_mode(
            host_kind,
            token,
            domains,
            tool_groups,
            preloaded_config,
            crate::core::runtime::CoreBootMode::Normal,
        )
        .await
    }

    /// [`init_with_config`](Self::init_with_config) with an explicit startup
    /// policy. Inspection mode uses the supplied config but skips every
    /// persistent workspace/bootstrap effect while retaining a real dispatch
    /// context.
    pub(crate) async fn init_with_boot_mode(
        host_kind: HostKind,
        token: &TokenSource,
        mut domains: crate::core::runtime::DomainSet,
        mut tool_groups: crate::tools::toolpacks::ToolGroups,
        preloaded_config: Option<crate::config::Config>,
        boot_mode: crate::core::runtime::CoreBootMode,
    ) -> anyhow::Result<(Arc<CoreContext>, bool, Option<crate::config::Config>)> {
        if boot_mode == crate::core::runtime::CoreBootMode::Inspection {
            anyhow::ensure!(
                preloaded_config.is_some(),
                "inspection boot requires a caller-supplied config"
            );
            anyhow::ensure!(
                matches!(token, TokenSource::Fixed(_)),
                "inspection boot requires a process-local fixed RPC bearer"
            );
        }
        log::debug!(
            "[core-context] init: host_kind={host_kind:?} domains={domains:?} \
             tool_groups={tool_groups:?}"
        );
        crate::core::runtime::mode::admit_core(host_kind, DEFAULT_CONTEXT.get().is_some())
            .map_err(|e| anyhow::anyhow!("[core-context] {e}"))?;
        // 1. Ensure all controllers are registered before anything dispatches.
        let _ = crate::core::all::all_registered_controllers();

        // 2. Load the master encryption key before any config/credential op that
        //    needs to decrypt secrets. No-op if already called (e.g. from
        //    run_core_from_args for the CLI).
        if boot_mode != crate::core::runtime::CoreBootMode::Inspection {
            if let Some(config) = &preloaded_config {
                crate::security::secret_storage::init_workspace(&config.workspace_dir);
            }
            crate::security::secret_storage::init_master_key().map_err(anyhow::Error::msg)?;
        }

        // 4. Seed the per-process RPC bearer. `Fixed` seeds the in-memory value
        //    directly (never touches the env); `EnvOrFile` reads
        //    OPENHUMAN_CORE_TOKEN or generates + writes {root}/core.token.
        //
        //    `has_operator_token` records whether an OPERATOR-supplied bearer
        //    exists (in-memory handoff or env var). The self-generated core.token
        //    file does NOT count — remote clients cannot read it — so it must not
        //    satisfy the public-bind safety check in `serve`.
        let has_operator_token = match token {
            TokenSource::Fixed(token) => {
                crate::core::auth::init_rpc_token_with_value(token)?;
                !token.trim().is_empty()
            }
            TokenSource::EnvOrFile => {
                // A caller-supplied config scopes the core's state, so a
                // self-generated bearer must land beside it rather than under
                // the operator's real `~/.openhuman` root — otherwise an
                // "ephemeral" harness still writes a `core.token` into the
                // operator's install. Fall back to the default root only when
                // no config was supplied.
                let token_dir = preloaded_config
                    .as_ref()
                    .map(|cfg| {
                        cfg.config_path
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| cfg.config_path.clone())
                    })
                    .unwrap_or_else(|| {
                        crate::config::default_root_openhuman_dir().unwrap_or_else(|_| {
                            dirs::home_dir()
                                .unwrap_or_else(|| std::path::PathBuf::from("."))
                                .join(".openhuman")
                        })
                    });
                crate::core::auth::init_rpc_token(&token_dir)?;
                std::env::var(crate::core::auth::CORE_TOKEN_ENV_VAR)
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .is_some()
            }
        };

        // 5. Resolve config once, then initialize workspace-bound stores
        //    (memory, attachments, people) with that exact workspace.
        // Kept for the context: `preloaded_config` is consumed below, and the
        // whole point is that handlers can reach it after boot.
        let embedder_config = preloaded_config.clone();
        let loaded = match preloaded_config {
            // A supplied config is authoritative: no disk read, no env overlay,
            // and no `Err` arm to reach, because there was nothing to fail.
            Some(cfg) => {
                log::debug!("[core-context] init: using caller-supplied config (scoped workspace)");
                Ok(cfg)
            }
            None if boot_mode == crate::core::runtime::CoreBootMode::SecretPreflight => {
                crate::config::Config::load_or_init_unsealed().await
            }
            None => crate::config::Config::load_or_init().await,
        };
        // `[components]` can only narrow what the host chose.
        let components = match &loaded {
            Ok(cfg) => crate::core::runtime::ComponentSet::from_config(&cfg.components),
            Err(_) => crate::core::runtime::ComponentSet::all(),
        };
        domains = components.narrow_domains(domains);
        tool_groups = components.narrow_tool_groups(tool_groups);
        log::debug!("[core-context] init: components={components:?} effective domains={domains:?}");
        let config = match loaded {
            Ok(cfg) => {
                if boot_mode != crate::core::runtime::CoreBootMode::Inspection {
                    init_stores(&cfg, domains).await;
                }
                Some(cfg)
            }
            Err(e) => {
                log::error!(
                    "[boot] workspace-bound store init SKIPPED — \
                     Config::load_or_init failed ({e:#}). Memory persistence is \
                     DISABLED for this run; no silent fallback to the default \
                     workspace (which would cause chunk loss / cross-workspace \
                     bleed-over). Fix config.toml or set OPENHUMAN_WORKSPACE to a \
                     writable path, then restart."
                );
                None
            }
        };
        // The desktop's restricted preflight loads config internally before
        // it can decrypt config secrets. Keep that resolved config available
        // as the context snapshot so successful decryption can replace it
        // before the normal router is opened.
        let latest_embedder_config = embedder_config
            .clone()
            .or_else(|| {
                (boot_mode == crate::core::runtime::CoreBootMode::SecretPreflight)
                    .then(|| config.clone())
                    .flatten()
            })
            .map(|config| {
                let snapshot = Arc::new(std::sync::Mutex::new(config));
                crate::config::Config::bind_context_snapshot(&snapshot);
                snapshot
            });
        let workspace_dir = config.as_ref().map(|cfg| cfg.workspace_dir.clone());

        // Construct the runtime context before bootstrap. Inspection contexts
        // stay local and no-store; normal hosts still publish the process
        // default before their session provider can request it.
        let ctx = if boot_mode == crate::core::runtime::CoreBootMode::Inspection {
            super::CoreContext::for_runtime_inspection(
                config
                    .clone()
                    .expect("inspection boot validated its preloaded config"),
                host_kind,
                domains,
                tool_groups,
            )
        } else {
            let context = Arc::new(CoreContext {
                host_kind,
                workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
                    workspace_dir: workspace_dir.clone(),
                    config_path: config.as_ref().map(|cfg| cfg.config_path.clone()),
                }))),
                domains,
                tool_groups,
                autonomy_tier_source: if embedder_config.is_some() {
                    super::AutonomyTierSource::Captured
                } else {
                    super::AutonomyTierSource::LiveConfig
                },
                autonomy_tier_ceiling: None,
                components,
                embedder_config,
                latest_embedder_config,
                user_skill_roots: true,
                extensions: Default::default(),
                turn_origin: None,
                channel_thread: None,
                session_agent: None,
                profile: None,
                profile_admission: Arc::new(()),
                agent: Default::default(),
                bare: false,
            });
            let _ = DEFAULT_CONTEXT.set(context.clone());
            context
        };

        // 6. Long-lived runtime infrastructure: event bus, domain subscribers,
        //    ledgers, agent-definition registry, live security policy, approval
        //    gate, socket manager. Idempotent (Once-guarded internally). Selected
        //    background jobs start later, from CoreRuntime::start_services(), after a transport binds
        //    succeeds.
        let runtime_config = config.clone();
        if boot_mode != crate::core::runtime::CoreBootMode::Inspection {
            crate::core::runtime::bootstrap::bootstrap_core_runtime(
                host_kind,
                config,
                domains,
                &crate::core::runtime::DispatchCtx::new(ctx.clone()),
                boot_mode == crate::core::runtime::CoreBootMode::SecretPreflight,
            )
            .await;
        }

        Ok((ctx, has_operator_token, runtime_config))
    }
}
