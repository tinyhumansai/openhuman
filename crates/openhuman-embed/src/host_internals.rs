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
            is_transient_http_status_code, is_transient_message_failure, report_error_or_expected,
            report_warning_message, ExpectedErrorKind, API_KEY_REJECTED_PREFIX,
            BACKEND_UNAVAILABLE_PREFIX, REPORT_ERROR_TRACING_TARGET,
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
        pub use openhuman_core::core::events::DomainEvent;
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
        pub use openhuman_core::core::session_expiry::is_session_expired_error;
    }
    pub mod shutdown {
        pub use openhuman_core::core::shutdown::{register, signal};
    }
    pub mod types {
        pub use openhuman_core::core::types::AppState;
    }
}

/// tinyhumans (backend transport, hosted client, team ops) and rpc (`/health`).
pub mod backend {
    pub use openhuman_core::backend::{
        base_url, flatten_authed_error, BackendApiError, BackendClient,
    };

    pub mod transport {
        pub use openhuman_core::backend::transport::{
            clear_backend_transport, resolve_backend_transport, BackendTransport,
            BackendTransportError, TransportProfile,
        };
    }
}

/// tinyhumans (every hosted controller loads its config) and rpc (server).
pub mod config {
    pub use openhuman_core::config::{
        active_workspace_dir, active_workspace_dir_cached, active_workspace_snapshot,
        load_config_with_timeout, workspace_handle, Config,
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
    pub mod rpc {
        pub use openhuman_core::config::rpc::load_config_with_timeout;
    }
    pub mod schema {
        pub mod cloud_providers {
            pub use openhuman_core::config::schema::cloud_providers::{
                endpoint_host, host_is_builtin_cloud_provider,
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
        // rpc: Socket.IO run-mode toggles.
        pub mod run_mode {
            pub use openhuman_core::agent::tinyagents::run_mode::{parse_mode_label, set_mode};
        }
    }
    // rpc: shutdown, plan review over Socket.IO, and the session store install.
    pub mod orchestration {
        pub use openhuman_core::agent::orchestration::release_background_completion_stores;
    }
    pub mod plan_review {
        pub mod gate {
            pub use openhuman_core::agent::plan_review::gate::global;
        }
    }
    pub mod session_store {
        pub use openhuman_core::agent::session_store::{
            context_workspace_dir, install, installed, restore, ReboundSessionStores,
            WorkspaceSessionStores,
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

/// tinyhumans: the team ops budget gate.
pub mod integrations {
    pub mod client {
        pub use openhuman_core::integrations::client::budget_gate;
    }
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
    pub mod credentials {
        pub use openhuman_core::security::credentials::{
            channel_credential_provider, AuthService, APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
        };
        pub mod ops {
            pub use openhuman_core::security::credentials::ops::{
                list_provider_credentials, store_provider_credentials,
            };
        }
        pub mod session_support {
            pub use openhuman_core::security::credentials::session_support::{
                build_session_state, current_session_is_local, resolve_backend_credential,
                BackendCredential, LOCAL_SESSION_BACKEND_UNAVAILABLE,
            };
        }
    }
    pub mod approval {
        pub use openhuman_core::security::approval::{ApprovalChatContext, ApprovalGate};
    }
    pub mod pairing {
        pub use openhuman_core::security::pairing::is_public_bind;
    }
}

/// tinyhumans: TLS, URL and redaction helpers for the backend client.
pub mod util {
    pub use openhuman_core::util::redact_url_for_log;

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

// The rest is the rpc server alone.

pub mod desktop {
    pub mod notifications {
        pub use openhuman_core::desktop::notifications::subscribe_core_notifications;
    }
    pub mod overlay {
        pub use openhuman_core::desktop::overlay::subscribe_attention_events;
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
}

pub mod storage {
    pub use openhuman_core::storage::{
        block_on, block_on_anyhow, clear, configured_url, driver_is_shared, install,
        install_for_url, installed, open, url_from, StorageBackend, STORAGE_URL_VAR,
    };

    /// The layout v1 -> v2 migration the host runs at boot when
    /// `[storage] auto_migrate_layout` is set.
    pub mod layout {
        pub use openhuman_core::storage::layout::auto_migrate;
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
    pub mod dictation_listener {
        pub use openhuman_core::voice::dictation_listener::{
            subscribe_dictation_events, subscribe_transcription_results,
        };
    }
    // The core carries the live-voice and dictation sockets only with voice and
    // the HTTP server compiled in. Where its features differ from embed's (a
    // workspace build unifies them separately), rpc still names these paths, so
    // embed supplies a socket-dropping stand-in rather than a missing item.
    pub mod live {
        pub mod ws {
            #[cfg(all(feature = "voice", feature = "http-server"))]
            pub use openhuman_core::voice::live::ws::handle_live_voice_ws;
            #[cfg(not(all(feature = "voice", feature = "http-server")))]
            pub async fn handle_live_voice_ws<S, X, C>(_socket: S, _ctx: X, _config: C) {}
        }
    }
    pub mod streaming {
        #[cfg(all(feature = "voice", feature = "http-server"))]
        pub use openhuman_core::voice::streaming::handle_dictation_ws;
        #[cfg(not(all(feature = "voice", feature = "http-server")))]
        pub async fn handle_dictation_ws<S, C>(_socket: S, _config: C) {}
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
        ChatRequestMetadata, GuardrailPayload, StartChatError, WebChannelEvent,
    };
}

#[cfg(test)]
#[path = "host_internals_tests.rs"]
mod tests;
