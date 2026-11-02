//! Core internals for the crates layered directly above embed
//! (`openhuman-tinyhumans`, `openhuman-rpc`), and nothing else.
//!
//! Those layers implement the backend transport, the hosted controllers and
//! the JSON-RPC server, which reach into the core by nature. Routing them
//! through this module keeps the dependency chain strict (they need no
//! `openhuman-core` dependency of their own) without inventing a public API
//! for every server internal.
//!
//! This is an explicit list of items, not a glob of modules: every entry is a
//! symbol one of the two consumers named in its group actually uses, and a new
//! reach-in is added here on purpose. The nested module layout mirrors the
//! core's paths so a consumer's `__host::a::b::item` reads like the core path.
//! Hosts (app, CLI, TUI) never see this module: the curated facade at the
//! crate root is theirs, and the layers above embed re-export no part of it
//! (`scripts/ci/check-crate-chain.mjs` enforces that).

pub use openhuman_core::run_core_from_args;

/// Shared by tinyhumans (hosted controllers) and rpc (`http_host`, `/health`).
pub mod core {
    pub use openhuman_core::core::{
        unwrap_rpc, ControllerSchema, FieldSchema, Outcome, StructuredRpcError, TypeSchema,
    };

    pub mod all {
        // tinyhumans: hosted controller extension; rpc: http_host + `/health`.
        pub use openhuman_core::core::all::{
            all_controller_schemas, all_http_method_schemas, cli_handler_for_namespace,
            namespace_description, register_controller_extension, rpc_method_from_parts,
            schema_for_rpc_method, ControllerExtension, ControllerFuture, DomainGroup,
            RegisteredController,
        };
    }

    /// Error classification shared by the hosted client (tinyhumans) and the
    /// RPC handler and classifier (rpc).
    pub mod observability {
        pub use openhuman_core::core::observability::{
            contains_transient_transport_phrase, expected_error_kind, is_api_key_rejected_message,
            is_session_expired_message, is_suppressed_usage_probe_backoff,
            is_transient_http_status_code, is_transient_message_failure, report_error,
            report_error_or_expected, report_warning_message, ExpectedErrorKind,
            API_KEY_REJECTED_PREFIX, BACKEND_UNAVAILABLE_PREFIX, REPORT_ERROR_TRACING_TARGET,
            USAGE_PROBE_BACKOFF_PREFIX,
        };
    }

    // rpc server only, below.
    pub mod auth {
        pub use openhuman_core::core::auth::{
            bearer_matches, get_rpc_token, init_rpc_token, init_rpc_token_with_value,
            verify_bearer_token, CORE_TOKEN_ENV_VAR,
        };
    }
    pub mod bus {
        pub use openhuman_core::core::bus::{init, BUS};
    }
    pub mod cli {
        pub use openhuman_core::core::cli::load_dotenv_for_cli;
    }
    pub mod dispatch {
        pub use openhuman_core::core::dispatch::{
            is_known_probe_method, unknown_method_name, UNKNOWN_METHOD_PREFIX,
        };
    }
    pub mod event_bind_tokens {
        pub use openhuman_core::core::event_bind_tokens::{consume, issue};
    }
    pub mod events {
        // `DomainEvent` is matched variant by variant by the Socket.IO bridge.
        pub use openhuman_core::core::events::{DomainEvent, KeyringAccessApprovalRequest};
    }
    pub mod invoke {
        pub use openhuman_core::core::invoke::{default_state, invoke_method};
    }
    pub mod params {
        pub use openhuman_core::core::params::{
            is_param_validation_error, missing_required_param_message, unknown_param_message,
        };
    }
    pub mod runtime {
        pub use openhuman_core::core::runtime::{
            current_tenant, is_saas, CoreContext, CoreRuntime, DispatchCtx, Mode, SaasConfig,
        };
        pub mod saas {
            pub use openhuman_core::core::runtime::saas::build;
        }
    }
    pub mod server_launcher {
        pub use openhuman_core::core::server_launcher::{install_server_launcher, ServeRequest};
    }
    pub mod session_expiry {
        pub use openhuman_core::core::session_expiry::{
            backend_status_means_session_expired, is_session_expired_error,
        };
    }
    pub mod shutdown {
        pub use openhuman_core::core::shutdown::{register, signal};
    }
    /// The generic extension slots every `register_*` below is built on;
    /// `CoreBuilder::extensions` takes an [`Extensions`](slots::Extensions).
    pub mod slots {
        pub use openhuman_core::core::slots::{Extensions, HookSlot, Registry};
    }
    pub mod types {
        pub use openhuman_core::core::types::AppState;
    }
}

/// The generic doctor hook for checks contributed by a layered host.
pub mod doctor {
    /// The read-only desktop profile resolver used by the RPC host's one-shot
    /// doctor boot.
    pub mod profile {
        pub use openhuman_core::doctor::profile::{
            resolve_desktop_profile_for_doctor, DesktopProfileError, DesktopProfileSelection,
        };
    }

    pub use openhuman_core::doctor::registry::registered_checks;
    pub use openhuman_core::doctor::{
        register_check_source, register_check_source_in, ComponentId, DoctorCheck,
        DoctorCheckSource, DoctorConfigSource, DoctorCtx, DoctorTier, Finding, FindingSeverity,
    };
}

/// tinyhumans (every hosted controller loads its config) and rpc (server).
pub mod config {
    pub use openhuman_core::config::{
        active_workspace_dir, active_workspace_dir_cached, active_workspace_snapshot,
        build_runtime_proxy_client_with_timeouts, load_config_with_timeout, workspace_handle,
        Config, PrivacyMode,
    };

    pub mod app_env {
        // tinyhumans: backend URL selection.
        pub use openhuman_core::config::app_env::{
            app_env_from_env, is_staging_app_env, APP_ENV_VAR, VITE_APP_ENV_VAR,
        };
    }
    pub mod ops {
        pub use openhuman_core::config::ops::{load_config_in, load_config_with_timeout};
    }
    pub mod migrations {
        pub use openhuman_core::config::migrations::run_pending;
    }
    pub mod rpc {
        pub use openhuman_core::config::rpc::load_config_with_timeout;
    }
    pub mod schema {
        // tinyhumans integrations: the Composio routing-mode strings.
        pub use openhuman_core::config::schema::{
            ModelRouteConfig, TeamModelConfig, COMPOSIO_MODE_BACKEND, COMPOSIO_MODE_DIRECT,
            COMPOSIO_MODE_DISABLED,
        };
        pub mod cloud_providers {
            pub use openhuman_core::config::schema::cloud_providers::{
                generate_provider_id, is_hosted_auth_style, is_hosted_cloud_identity,
                register_hosted_cloud_preset, register_hosted_cloud_preset_in,
                register_hosted_cloud_provider_policy, register_hosted_cloud_provider_policy_in,
                AuthStyle, BuiltinCloudProvider, CloudProviderCreds, HostedAuthStyleClassifier,
                HostedCloudIdentityClassifier,
            };
        }
    }
}

/// tinyhumans: the Jev tool ranker and the jev route.
pub mod agent {
    pub mod tinyagents {
        pub mod discovery {
            pub use openhuman_core::agent::tinyagents::discovery::{
                embedding_provider_is_usable, embedding_tool_ranker, embedding_tool_ranker_in,
                install_tool_ranker, installed_tool_ranker,
            };
        }
        // tinyhumans integrations: the turn's approval context for a tool call.
        pub mod host {
            pub mod run_context {
                pub use openhuman_core::agent::tinyagents::host::run_context::approval_for_tool;
            }
        }
        // rpc: Socket.IO run-mode toggles.
        pub mod run_mode {
            pub use openhuman_core::threads::{parse_mode_label, set_mode};
        }
    }
    // rpc: shutdown, plan review over Socket.IO, and the session store install.
    pub mod orchestration {
        pub use openhuman_core::agent::orchestration::release_background_completion_stores;
    }
    pub mod plan_review {
        pub use openhuman_core::plan_review::host::{
            parked_review_for_socket, socket_scope_for_dispatch, SocketReviewScope,
        };
    }
    pub mod session_store {
        pub use openhuman_core::storage::sessions::{
            context_workspace_dir, install, installed, restore, ReboundSessionStores,
            TenantSessionStores, WorkspaceSessionStores,
        };
    }
}

/// tinyhumans: the channel-link controller schema.
pub mod channels {
    pub mod contract_schema {
        pub use openhuman_core::channels::contract_schema::contract_controller_schema;
    }
}

/// tinyhumans: embeddings and provider-key lookup for the Jev ranker; rpc: the
/// OpenAI-compatible HTTP router.
pub mod inference {
    pub mod embedding_host {
        pub use openhuman_core::inference::embedding_host::{
            default_embedding_provider_for_chat, default_embedding_provider_with_config,
            SharedEmbeddingModel,
        };
    }
    pub mod provider {
        pub mod factory {
            pub use openhuman_core::inference::provider::factory::lookup_key_for_slug;
        }
    }
    pub mod http {
        #[cfg(feature = "http-server")]
        pub use openhuman_core::inference::http::router;
        pub use openhuman_core::inference::http::EXTERNAL_OPENAI_COMPAT_PROVIDER;
    }
}

/// Native memory bearer types used by hosted backend transport adapters.
pub mod memory {
    pub use openhuman_core::memory::engine::{BearerSource, CallbackBearerSource, MemoryError};
}

/// tinyhumans (credentials for the hosted client) and rpc (server auth,
/// Socket.IO approvals, bind policy).
pub mod security {
    /// Host-supplied secret storage and encryption-key sources.
    pub mod secret_storage {
        pub use openhuman_core::security::secret_storage::{
            clear_master_key_source, clear_secret_backend, init_workspace,
            install_master_key_source, install_secret_backend, installed_master_key_source,
            installed_secret_backend, is_available, store::workspace_dir_for_file_backend,
            EncryptedFileBackend, MasterKeySource, SecretBackend, SecretStorageError,
        };
    }
    pub mod approval {
        pub use openhuman_core::security::approval::{
            ApprovalChatContext, ApprovalGate, ApprovalTurnContext,
        };
    }
    // tinyhumans integrations: the autonomy policy the file-storage tools check,
    // and the live policy its local-only egress tests install.
    pub use openhuman_core::security::{AutonomyLevel, SecurityPolicy};
    pub mod live_policy {
        pub use openhuman_core::security::live_policy::install;
    }
    pub mod pairing {
        pub use openhuman_core::security::pairing::is_public_bind;
    }
    /// TinyHumans' hosted device-tunnel adapter and the semantic device events
    /// it forwards into the core.
    pub mod devices {
        pub use openhuman_core::security::devices::tunnel_client::{
            global_tunnel_client_port, publish_peer_status, publish_tunnel_frame,
            set_global_tunnel_client_port, TunnelClientPort, TunnelRegistration,
        };
    }
}

/// Host credential custody and read-side auth helpers used by tinyhumans and rpc.
pub mod credentials {
    pub use openhuman_core::credentials::{
        channel_credential_provider, clear_provider_credential_source,
        install_provider_credential_source, provider_credential_source, state_dir_from_config,
        AuthService, ProviderCredentialSource, ProviderOAuthStart, ProviderOAuthTokens,
        APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
    };
    pub mod openai_oauth {
        pub use openhuman_core::credentials::openai_oauth::{
            complete_openai_oauth, disconnect_openai_oauth, import_openai_oauth_from_codex_cli,
            openai_oauth_status, persist_openai_oauth_token, persist_openai_oauth_tokens,
            start_openai_oauth,
        };
    }
    pub mod ops {
        pub use openhuman_core::credentials::ops::{
            list_provider_credentials, store_provider_credentials,
        };
    }
    pub mod api_key {
        pub use openhuman_core::credentials::api_key::{
            get_api_key, get_api_key_in, get_api_key_read_only, store_api_key, store_api_key_in,
        };
    }
    pub mod session_state {
        pub use openhuman_core::credentials::session_state::{
            build_session_state, check_session_backend, classify_session_token,
            current_session_is_local, get_session_token, is_local_session_token,
            load_app_session_profile, load_app_session_profile_read_only, local_session_user_id,
            normalize_session_backend, parse_fields_value, profile_name_or_default,
            require_live_session_token, session_state_from_profile, session_token_for_config,
            session_token_from_profile, summarize_auth_profile, user_id_from_jwt_claims,
            CredentialKind, SessionTokenCheck, LOCAL_SESSION_USER_ID, SESSION_EXPIRES_AT_META,
            SESSION_ISSUING_BACKEND_META,
        };
    }
    pub mod backend_credential {
        pub use openhuman_core::credentials::backend_credential::{
            ambient_config_has_api_key, backend_bearer_secret, credential_fingerprint,
            credential_fingerprint_matches, direct_backend_credential, has_backend_credential,
            publish_local_session_expiry, resolve_backend_credential, BackendCredential,
            LOCAL_SESSION_BACKEND_UNAVAILABLE, LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE,
        };
    }
}

/// tinyhumans: TLS, URL and redaction helpers for the backend client.
pub mod util {
    pub use openhuman_core::util::{redact_url_for_log, truncate_at_byte_boundary};

    pub mod redact {
        pub use openhuman_core::util::redact::redact_url_for_log;
    }
    pub mod tls {
        pub use openhuman_core::util::tls::tls_client_builder;
    }
    pub mod url {
        pub use openhuman_core::util::url::{
            host_is_local, join_url, normalize_api_base_url, normalize_backend_api_base_url,
        };
    }
}

// ---------------------------------------------------------------------------
// Extension points (tinyhumans). Generic, id-keyed registries the hosted layer
// fills at install time; the core knows nothing about what it is handed. Each
// `register_*` has an `_in` variant that binds to a `CoreBuilder::extensions`
// set instead of the process.
// ---------------------------------------------------------------------------

/// Tool sources: tools an outside crate appends to the agent catalogue.
pub mod tool_sources {
    pub use openhuman_core::tools::sources::{
        register_tool_source, register_tool_source_at, register_tool_source_in,
        registered_tool_source_placements, unregister_tool_source, ToolSourceCtx,
        ToolSourceFactory, ToolSourcePlacement, ANCHOR_AFTER_DELEGATE, ANCHOR_AFTER_SEARCH,
    };
}

/// The artifact publisher slot: the service that shares an artifact version.
pub mod artifact_publisher {
    pub use openhuman_core::artifacts::{
        register_artifact_publisher, register_artifact_publisher_in, unregister_artifact_publisher,
        ArtifactPublisherBinder, ArtifactPublisherFactory,
    };
    // tinyhumans file-storage publisher: the protected-path guard it re-checks
    // before upload, and the store its tests save artifacts into.
    pub use openhuman_core::artifacts::tiny_store::{version_files_dir, versioned_store};
}

/// Egress disclosure, for hosted calls that leave the machine.
pub mod egress {
    pub use openhuman_core::security::egress::{
        data_class, emit_external_transfer, emit_external_transfer_in, enforce_egress,
        enforce_egress_blocking, DataKind, EgressDescriptor, EgressLedger, EgressReason,
        IdentificationRisk, PrivacyClass,
    };
}

/// Config enrichers and the resolved fields they fill.
pub mod config_enrichers {
    pub use openhuman_core::config::schema::resolved_endpoints::ResolvedEndpoints;
    pub use openhuman_core::config::{
        register_enricher, register_enricher_in, run_enrichers, unregister_enricher, ConfigEnricher,
    };
}

/// Composio route builders (`composio.mode` -> route).
pub mod composio {
    pub use openhuman_core::integrations::composio::client::{
        register_composio_route_builder, unregister_composio_route_builder, ComposioRoute,
        ComposioRouteBuilder,
    };
    // The proxy route's target (origin + bearer) for `composio.mode = backend`.
    #[cfg(feature = "modules")]
    pub use openhuman_core::modules::connectors::{
        register_proxy_target, unregister_proxy_target, ProxyTarget, ProxyTargetResolver,
    };
}

/// Media-generation tool building over host-supplied generators (tinyhumans).
#[cfg(feature = "media")]
pub mod media {
    pub mod generation {
        pub use openhuman_core::media::generation::{
            media_tools_for_config, GeneratorRouter, MediaGenerators, MediaReferenceAllowlist,
        };
    }
}

/// The wallet module's authenticated backend hook (tinyhumans web3).
#[cfg(feature = "web3")]
pub mod wallet_backend {
    pub use openhuman_core::modules::wallet_backend::{
        register_wallet_backend, register_wallet_backend_in, unregister_wallet_backend,
        WalletBackendCall, WalletBackendFuture, WalletBackendHandler,
    };
}

/// Model-provider registries: chat models, embeddings, catalogs.
pub mod inference_registry {
    pub use openhuman_core::inference::provider::factory::PROVIDER_OPENHUMAN;
    pub use openhuman_core::inference::registry::{
        classify_backend_error, provider_default_model, provider_default_pricing,
        provider_model_context_window, provider_model_is_alias, provider_model_is_raw,
        provider_model_role, provider_model_supports_vision, provider_request_without_reasoning,
        register_backend_error_classifier, register_backend_error_classifier_in, register_catalog,
        register_catalog_in, register_chat_model, register_chat_model_in,
        register_config_normalizer, register_config_normalizer_in, register_embedding,
        register_embedding_in, register_inspection_catalog, register_inspection_catalog_in,
        register_inspection_chat_model, register_inspection_chat_model_in,
        register_inspection_embedding, register_inspection_embedding_in, resolve_provider_model,
        unregister_backend_error_classifier, unregister_catalog, unregister_chat_model,
        unregister_embedding, unregister_inspection_catalog, unregister_inspection_chat_model,
        BackendErrorClassification, BackendErrorClassifier, BackendErrorKind, CatalogCallContext,
        CatalogHostRequest, ConfigNormalizer, EmbeddingCallContext, InspectionCatalogError,
        InspectionCatalogListing, InspectionCatalogRequest, InspectionChatModelBuild,
        InspectionChatModelFactory, InspectionEmbeddingFactory, PreparedInspectionCatalog,
        PreparedInspectionChatModelFactory, ProviderContext,
    };
    pub use tinyinference_llm::providers::overrides::{
        BoxFuture, CatalogEntry, CatalogLimits, CatalogListing, CatalogRequest, ChatModelEntry,
        ChatModelFactory, ChatModelRequest, EmbeddingFactory, EmbeddingRequest,
        ProviderModelMetadata, ProviderOverrides, ReadinessProbe,
    };
}

/// Memory engines by id, and the free-background-work predicate.
pub mod memory_engines {
    pub use openhuman_core::memory::engine::{
        build_cached, rooted, BearerSource, Binding, BoundEngine, EngineCredential, EngineHealth,
        EngineSettings, TINYHUMANS_ENGINE,
    };
    pub use openhuman_core::memory::engine_registry::{
        bound_engine, register_memory_engine, register_memory_engine_in,
        register_memory_health_probe, register_memory_health_probe_in,
        register_read_only_engine_factory, register_read_only_engine_factory_in,
        unregister_memory_engine, EngineBuilder, EngineEntry, EngineFuture, EngineRequest,
        FreeBackground, ReadOnlyEngineError, ReadOnlyEngineFactory, ReadOnlyHealthProbe,
    };
}

/// The channel callback slot.
#[cfg(feature = "channels")]
pub mod channel_callback {
    pub use openhuman_core::channels::callback_slot::{
        register_channel_callback, register_channel_callback_in, unregister_channel_callback,
        CallbackFuture, ChannelCallback, ChannelCallbackCtx,
    };
}

/// Voice provider registries: STT, TTS and live sessions by slug.
#[cfg(feature = "voice")]
pub mod voice_registry {
    pub use openhuman_core::config::schema::voice_live::{
        LIVE_PROVIDER_ELEVENLABS_HOSTED, LIVE_PROVIDER_GEMINI_HOSTED,
    };
    pub use openhuman_core::voice::factory::registry::{
        register_stt_provider, register_stt_provider_in, register_tts_provider,
        register_tts_provider_in, unregister_stt_provider, unregister_tts_provider,
        SttProviderFactory, TtsProviderFactory,
    };
    pub use openhuman_core::voice::factory::{
        create_stt_provider, create_tts_provider, SttProvider, SttResult, TtsProvider,
    };
    pub use openhuman_core::voice::live::ops::live_providers;
    pub use openhuman_core::voice::live::registry::{
        register_live_provider, register_live_provider_in, unregister_live_provider, ConfigureFn,
        ConfiguredFn, LiveFuture, LiveProviderEntry, LiveProviderPlacement, PrepareFn,
    };
    pub use openhuman_core::voice::live::types::{LiveProviderInfo, LiveProviderKind};
    pub use openhuman_core::voice::live::{
        LiveVoiceError, PreparedProvider, GEMINI_LANGUAGES, GEMINI_VOICES,
    };
}

// The rest is the rpc server alone.

pub mod desktop {
    pub mod notifications {
        pub use openhuman_core::desktop::notifications::subscribe_core_notifications;
    }
}

pub mod mcp {
    pub mod registry {
        pub mod oauth {
            pub use openhuman_core::mcp::registry::oauth::complete;
        }
    }
}

pub mod platform {
    /// tinyhumans (managed inference): the signed-out gate.
    pub mod llm_gate {
        pub use openhuman_core::platform::llm_gate::is_signed_out;
    }

    pub mod connectivity {
        pub mod rpc {
            pub use openhuman_core::platform::connectivity::rpc::{
                pick_listen_port_for_host_with, OccupiedByCore,
            };
        }
    }
    pub mod health {
        pub use openhuman_core::platform::health::{snapshot, verdict};
    }

    /// tinyhumans: read-only view of the live Socket.IO connection state.
    pub mod socket {
        pub use openhuman_core::platform::socket::models::ConnectionStatus;
        pub use openhuman_core::platform::socket::models::SocketState;
        pub use openhuman_core::platform::socket::{
            dispatch_socket_event, global_socket_manager, global_socket_port,
            set_global_socket_manager, set_global_socket_port, SocketIdentityRebindGuard,
            SocketPort, TokenProvider,
        };
    }
}

pub mod skills {
    pub mod webhooks {
        pub use openhuman_core::skills::webhooks::WebhookRouter;
    }
}

pub mod storage {
    pub use openhuman_core::storage::{
        block_on, block_on_anyhow, clear, configured_url, driver_is_shared, install,
        install_for_url, installed, installed_url, open, url_from, StorageBackend, StorageUrl,
        STORAGE_URL_VAR,
    };

    /// The installed storage backend's scoped secret-store interface used by
    /// host-owned short-lived flows such as provider OAuth.
    pub mod secrets {
        pub use openhuman_core::storage::secrets::{current, MemorySecrets, SecretStore};
    }

    /// The layout v1 -> v2 migration the host runs at boot when
    /// `[storage] auto_migrate_layout` is set. The core builds it only with
    /// `legacy-import`.
    #[cfg(feature = "legacy-import")]
    pub mod layout {
        #[cfg(feature = "legacy-import")]
        pub use openhuman_core::storage::layout::auto_migrate;

        /// Return `false` when legacy layout import is not part of this build.
        ///
        /// The RPC session-store host calls this before resolving its storage
        /// URL. In builds without `legacy-import`, there is no layout migration
        /// to run and the configured workspace remains unchanged.
        #[cfg(not(feature = "legacy-import"))]
        pub async fn auto_migrate(_config: &openhuman_core::config::Config) -> bool {
            false
        }
    }
}

pub mod profiles {
    pub mod gateway {
        pub use openhuman_core::profiles::gateway::{
            resolve_scope, sign, verify, GatewayRefusal, GatewayScope, HeldBy, PROFILE_HELD,
            PROFILE_OWNER_HEADER, USER_HEADER, USER_SIG_HEADER,
        };
    }
    pub mod host {
        pub use openhuman_core::profiles::host::host;
    }
}

pub mod voice {
    // Where feature unification differs between embed and core, rpc still names
    // the live-voice path; embed supplies a socket-dropping stand-in.
    pub mod live {
        pub mod ws {
            #[cfg(all(feature = "voice", feature = "http-server"))]
            pub use openhuman_core::voice::live::ws::handle_live_voice_ws;
            #[cfg(not(all(feature = "voice", feature = "http-server")))]
            pub async fn handle_live_voice_ws<S, X, C>(_socket: S, _ctx: X, _config: C) {}
        }
    }
}

pub mod web3 {
    pub mod wallet {
        pub use openhuman_core::web3::wallet::WALLET_NOT_CONFIGURED_MESSAGE;
    }
}

pub mod web_chat {
    pub use openhuman_core::web_chat::{
        approval_request_event, cancel_chat_scoped, plan_review_request_event,
        publish_web_channel_event, start_chat, subscribe_web_channel_events, unix_epoch_ms,
        ChatRequestMetadata, GuardrailPayload, ScopedWebEvent, StartChatError, WebChannelEvent,
        WebEventContext,
    };
}

#[cfg(test)]
#[path = "host_internals_tests.rs"]
mod tests;
