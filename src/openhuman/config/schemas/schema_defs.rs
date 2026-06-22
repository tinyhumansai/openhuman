use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::helpers::{json_output, optional_bool, optional_json, optional_string};

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "get_config" => ControllerSchema {
            namespace: "config",
            function: "get",
            description: "Read persisted config snapshot and resolved paths.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "snapshot",
                ty: TypeSchema::Json,
                comment: "Config snapshot with workspace and config paths.",
                required: true,
            }],
        },
        "get_client_config" => ControllerSchema {
            namespace: "config",
            function: "get_client_config",
            description: "Read safe client-facing config fields (api_url, feature flags). No secrets.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "api_url",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Configured OpenHuman product backend URL, if any.",
                    required: false,
                },
                FieldSchema {
                    name: "inference_url",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Custom OpenAI-compatible LLM endpoint, if any. When set together with an api_key, inference goes direct to this URL.",
                    required: false,
                },
                FieldSchema {
                    name: "default_model",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Default model identifier.",
                    required: false,
                },
                FieldSchema {
                    name: "app_version",
                    ty: TypeSchema::String,
                    comment: "OpenHuman core version.",
                    required: true,
                },
                FieldSchema {
                    name: "api_key_set",
                    ty: TypeSchema::Bool,
                    comment: "True when a custom backend api_key is stored locally. The key itself is never returned over RPC.",
                    required: true,
                },
                FieldSchema {
                    name: "model_routes",
                    ty: TypeSchema::Json,
                    comment: "Persisted task-hint -> model id pairs the core router will obey. Empty when the OpenHuman built-in router is active.",
                    required: true,
                },
            ],
        },
        "update_model_settings" => ControllerSchema {
            namespace: "config",
            function: "update_model_settings",
            description: "Update model and backend connection settings, including a custom OpenAI-compatible backend (api_url + api_key).",
            inputs: vec![
                optional_string("api_url", "OpenHuman product backend URL (auth/billing/voice). Almost always left blank; the inference URL is a separate `inference_url` field."),
                optional_string("inference_url", "Custom OpenAI-compatible LLM endpoint. When set together with `api_key`, inference goes direct to this URL instead of the OpenHuman backend. Pass an empty string to clear."),
                optional_string("api_key", "Optional API key for the configured inference endpoint. Pass an empty string to clear a previously stored key."),
                optional_string("default_model", "Default model id."),
                FieldSchema {
                    name: "default_temperature",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Default model temperature.",
                    required: false,
                },
                FieldSchema {
                    name: "model_routes",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional list of {hint, model} pairs mapping task hints (reasoning, agentic, coding, summarization) to provider-specific model ids. Replaces config.model_routes wholesale; send [] to clear (e.g. when switching back to the OpenHuman built-in router).",
                    required: false,
                },
                FieldSchema {
                    name: "cloud_providers",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional list of cloud provider entries {id, slug, label, endpoint, auth_style}. API keys are stored separately via cloud_provider_set_key. Replaces config.cloud_providers wholesale.",
                    required: false,
                },
                optional_string("primary_cloud", "id of the cloud_providers entry used when a workload routes to 'cloud'. Empty string clears."),
                optional_string("chat_provider", "Provider string for direct conversational chat workloads."),
                optional_string("reasoning_provider", "Provider string for the main reasoning workload (e.g. 'cloud', 'ollama:llama3.1:8b', 'openai:gpt-4o')."),
                optional_string("agentic_provider", "Provider string for sub-agent / tool-loop workloads."),
                optional_string("coding_provider", "Provider string for code-generation workloads."),
                optional_string("vision_provider", "Provider string for the vision / multimodal workload (managed default: vision-v1)."),
                optional_string("memory_provider", "Provider string for memory-tree extract + summarise."),
                optional_string("embeddings_provider", "Provider string for embedding generation."),
                optional_string("heartbeat_provider", "Provider string for the heartbeat background-reasoning loop."),
                optional_string("learning_provider", "Provider string for learning / reflection passes."),
                optional_string("subconscious_provider", "Provider string for subconscious evaluation."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_memory_settings" => ControllerSchema {
            namespace: "config",
            function: "update_memory_settings",
            description: "Update memory backend and embedding settings.",
            inputs: vec![
                optional_string("backend", "Memory backend identifier."),
                FieldSchema {
                    name: "auto_save",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Enable auto-save.",
                    required: false,
                },
                optional_string("embedding_provider", "Embedding provider identifier."),
                optional_string("embedding_model", "Embedding model identifier."),
                FieldSchema {
                    name: "embedding_dimensions",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Embedding dimensions.",
                    required: false,
                },
                optional_string(
                    "memory_window",
                    "Stepped long-term memory window preset: minimal | balanced | extended | maximum.",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_screen_intelligence_settings" => ControllerSchema {
            namespace: "config",
            function: "update_screen_intelligence_settings",
            description: "Update screen intelligence runtime settings.",
            inputs: vec![
                optional_bool("enabled", "Enable screen intelligence."),
                optional_string("capture_policy", "Capture policy mode."),
                optional_string("policy_mode", "Policy mode override."),
                FieldSchema {
                    name: "baseline_fps",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Baseline capture FPS.",
                    required: false,
                },
                optional_bool("vision_enabled", "Enable vision analysis."),
                optional_bool("autocomplete_enabled", "Enable autocomplete integration."),
                optional_bool(
                    "use_vision_model",
                    "Use a vision LLM for screenshot analysis (false = OCR + text LLM).",
                ),
                optional_bool("keep_screenshots", "Keep screenshots on disk after vision processing."),
                FieldSchema {
                    name: "allowlist",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Allowed app list.",
                    required: false,
                },
                FieldSchema {
                    name: "denylist",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Denied app list.",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_runtime_settings" => ControllerSchema {
            namespace: "config",
            function: "update_runtime_settings",
            description: "Update runtime execution strategy settings.",
            inputs: vec![
                optional_string("kind", "Runtime kind."),
                optional_bool("reasoning_enabled", "Enable reasoning mode."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_autonomy_settings" => ControllerSchema {
            namespace: "config",
            function: "get_autonomy_settings",
            description: "Get the agent access-mode settings (autonomy level, workspace confinement, trusted roots, command allow-list, forbidden paths).",
            inputs: vec![],
            outputs: vec![json_output("autonomy", "Current [autonomy] config block.")],
        },
        "update_autonomy_settings" => ControllerSchema {
            namespace: "config",
            function: "update_autonomy_settings",
            description: "Update the agent access mode: autonomy level, workspace confinement, trusted-roots allow-list, command allow-list, forbidden paths, and OS-install permission. Applies live to active sessions.",
            inputs: vec![
                optional_string("level", "Autonomy level: readonly | supervised | full."),
                optional_bool("workspace_only", "Confine file/path access to the workspace directory."),
                FieldSchema {
                    name: "allowed_commands",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                    comment: "Replace the shell command allow-list (array of base command names).",
                    required: false,
                },
                FieldSchema {
                    name: "forbidden_paths",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                    comment: "Replace the forbidden-paths denylist (array of path prefixes).",
                    required: false,
                },
                FieldSchema {
                    name: "trusted_roots",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Replace the trusted-roots allow-list: array of {path, access: read|readwrite}. Grants access outside the workspace; credential dirs (~/.ssh, ~/.gnupg, ~/.aws) stay blocked regardless.",
                    required: false,
                },
                optional_bool("allow_tool_install", "Allow the agent to install OS packages via install_tool (intended for Full mode)."),
                FieldSchema {
                    name: "max_actions_per_hour",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Rate limit for side-effecting actions per hour.",
                    required: false,
                },
                FieldSchema {
                    name: "auto_approve",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                    comment: "Replace the \"Always allow\" allowlist (array of tool names the agent runs without an approval prompt). Empty array clears it.",
                    required: false,
                },
                optional_bool("require_task_plan_approval", "Require approval before an agent executes a task-board plan."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_agent_settings" => ControllerSchema {
            namespace: "config",
            function: "get_agent_settings",
            description: "Read agent execution settings: the action/tool wall-clock timeout, the runtime-effective value, and whether the OPENHUMAN_TOOL_TIMEOUT_SECS env var overrides it.",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "Agent settings: agent_timeout_secs, effective_timeout_secs, env_override, min_timeout_secs, max_timeout_secs.",
            )],
        },
        "update_agent_settings" => ControllerSchema {
            namespace: "config",
            function: "update_agent_settings",
            description: "Update agent execution settings. Currently the action/tool wall-clock timeout (seconds). Applies to the next tool call without a restart; the OPENHUMAN_TOOL_TIMEOUT_SECS env var still overrides it when set.",
            inputs: vec![FieldSchema {
                name: "agent_timeout_secs",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Wall-clock timeout for a single tool/action execution, in seconds (1–3600). Extend this when large local models are interrupted before finishing.",
                required: false,
            }],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_browser_settings" => ControllerSchema {
            namespace: "config",
            function: "update_browser_settings",
            description: "Update browser automation settings.",
            inputs: vec![optional_bool("enabled", "Enable browser integration.")],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "update_local_ai_settings" => ControllerSchema {
            namespace: "config",
            function: "update_local_ai_settings",
            description:
                "Update the local AI runtime master switch and per-feature usage flags.",
            inputs: vec![
                optional_bool(
                    "runtime_enabled",
                    "Master switch — when false, no subsystem uses the selected local AI runtime.",
                ),
                optional_bool(
                    "opt_in_confirmed",
                    "MVP opt-in marker. Bootstrap hard-overrides to disabled when this is false, \
                     regardless of `runtime_enabled`. Set in tandem with `runtime_enabled` from the \
                     unified AI panel.",
                ),
                optional_string(
                    "provider",
                    "Local provider identifier. Supported values: ollama, lm_studio, omlx.",
                ),
                optional_json(
                    "base_url",
                    "Provider base URL string, or null to clear. For LM Studio this defaults to http://localhost:1234/v1.",
                ),
                optional_string(
                    "api_key",
                    "Bearer credential for keyed local runtimes such as OMLX. Pass an empty string to clear.",
                ),
                optional_string("model_id", "Default local chat model identifier."),
                optional_string("chat_model_id", "Local chat model identifier."),
                optional_bool(
                    "usage_embeddings",
                    "Use the local model for embedding generation (when runtime_enabled).",
                ),
                optional_bool(
                    "usage_heartbeat",
                    "Use the local model inside the heartbeat loop (when runtime_enabled).",
                ),
                optional_bool(
                    "usage_learning_reflection",
                    "Use the local model for learning/reflection passes (when runtime_enabled).",
                ),
                optional_bool(
                    "usage_subconscious",
                    "Use the local model for subconscious evaluation (when runtime_enabled).",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "resolve_api_url" => ControllerSchema {
            namespace: "config",
            function: "resolve_api_url",
            description: "Resolve effective API base URL using config/env/default from core.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "api_url",
                ty: TypeSchema::String,
                comment: "Resolved backend API URL.",
                required: true,
            }],
        },
        "get_runtime_flags" => ControllerSchema {
            namespace: "config",
            function: "get_runtime_flags",
            description: "Read environment-driven runtime flags.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "flags",
                ty: TypeSchema::Ref("RuntimeFlagsOut"),
                comment: "Runtime flag state.",
                required: true,
            }],
        },
        "set_browser_allow_all" => ControllerSchema {
            namespace: "config",
            function: "set_browser_allow_all",
            description: "Disable browser allow-all mode, or enable it only when operator opt-in is present.",
            inputs: vec![FieldSchema {
                name: "enabled",
                ty: TypeSchema::Bool,
                comment: "Whether to enable browser allow-all mode. Runtime enable is refused unless OPENHUMAN_BROWSER_ALLOW_ALL_RPC_ENABLE=1.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "flags",
                ty: TypeSchema::Ref("RuntimeFlagsOut"),
                comment: "Updated runtime flag state.",
                required: true,
            }],
        },
        "workspace_onboarding_flag_exists" => ControllerSchema {
            namespace: "config",
            function: "workspace_onboarding_flag_exists",
            description: "Check if onboarding flag file exists in workspace.",
            inputs: vec![FieldSchema {
                name: "flag_name",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional onboarding flag name override.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "exists",
                ty: TypeSchema::Bool,
                comment: "True when the flag file is present.",
                required: true,
            }],
        },
        "workspace_onboarding_flag_set" => ControllerSchema {
            namespace: "config",
            function: "workspace_onboarding_flag_set",
            description: "Create or remove the onboarding flag file in workspace.",
            inputs: vec![
                FieldSchema {
                    name: "flag_name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional onboarding flag name override.",
                    required: false,
                },
                FieldSchema {
                    name: "value",
                    ty: TypeSchema::Bool,
                    comment: "True to create, false to remove.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "exists",
                ty: TypeSchema::Bool,
                comment: "True when the flag file is present after the operation.",
                required: true,
            }],
        },
        "update_analytics_settings" => ControllerSchema {
            namespace: "config",
            function: "update_analytics_settings",
            description: "Enable or disable anonymized analytics and error reporting.",
            inputs: vec![optional_bool(
                "enabled",
                "Enable anonymized analytics and crash reports.",
            )],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_analytics_settings" => ControllerSchema {
            namespace: "config",
            function: "get_analytics_settings",
            description: "Read current analytics settings.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "enabled",
                ty: TypeSchema::Bool,
                comment: "Whether anonymized analytics is enabled.",
                required: true,
            }],
        },
        "get_dashboard_settings" => ControllerSchema {
            namespace: "config",
            function: "get_dashboard_settings",
            description: "Read dashboard settings, including the local architecture diagram viewer.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "dashboard",
                ty: TypeSchema::Json,
                comment: "Current [dashboard] config block.",
                required: true,
            }],
        },
        "update_meet_settings" => ControllerSchema {
            namespace: "config",
            function: "update_meet_settings",
            description:
                "Update Meeting Assistant settings: auto-join, post-call summary, listen-only, transcript ingestion, and the orchestrator-handoff privacy gate.",
            inputs: vec![
                optional_bool(
                    "auto_orchestrator_handoff",
                    "When true, ending a Meet call hands the transcript to the orchestrator for proactive follow-up actions.",
                ),
                optional_string(
                    "auto_join_policy",
                    "Calendar auto-join policy: ask_each_time | always | never.",
                ),
                optional_string(
                    "auto_summarize_policy",
                    "Post-call summary policy: ask | always | never.",
                ),
                optional_bool(
                    "listen_only_default",
                    "When true, the bot joins in listen-only mode (mic muted).",
                ),
                optional_bool(
                    "ingest_backend_transcripts",
                    "When true, backend-bot meeting transcripts are ingested into memory.",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_meet_settings" => ControllerSchema {
            namespace: "config",
            function: "get_meet_settings",
            description: "Read current Meeting Assistant settings.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "auto_orchestrator_handoff",
                    ty: TypeSchema::Bool,
                    comment: "Whether the orchestrator handoff fires on Meet call end.",
                    required: true,
                },
                FieldSchema {
                    name: "auto_join_policy",
                    ty: TypeSchema::String,
                    comment: "Calendar auto-join policy: ask_each_time | always | never.",
                    required: true,
                },
                FieldSchema {
                    name: "auto_summarize_policy",
                    ty: TypeSchema::String,
                    comment: "Post-call summary policy: ask | always | never.",
                    required: true,
                },
                FieldSchema {
                    name: "listen_only_default",
                    ty: TypeSchema::Bool,
                    comment: "Whether the bot joins mic-muted (listen-only).",
                    required: true,
                },
                FieldSchema {
                    name: "ingest_backend_transcripts",
                    ty: TypeSchema::Bool,
                    comment: "Whether backend-bot transcripts are ingested into memory.",
                    required: true,
                },
            ],
        },
        "update_search_settings" => ControllerSchema {
            namespace: "config",
            function: "update_search_settings",
            description: "Update search engine selection and BYO API credentials.",
            inputs: vec![
                optional_string(
                    "engine",
                    "Active engine: managed | parallel | brave | querit.",
                ),
                FieldSchema {
                    name: "max_results",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum results per query (1-20).",
                    required: false,
                },
                FieldSchema {
                    name: "timeout_secs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Per-request timeout in seconds (1-120).",
                    required: false,
                },
                optional_string(
                    "parallel_api_key",
                    "Parallel API key (empty string clears the stored key).",
                ),
                optional_string(
                    "brave_api_key",
                    "Brave Search API key (empty string clears the stored key).",
                ),
                optional_string(
                    "querit_api_key",
                    "Querit API key (empty string clears the stored key).",
                ),
                FieldSchema {
                    name: "allowed_domains",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Websites the assistant may open/read (web_fetch/curl). Exact hosts match their subdomains; \"*\" allows all public sites; empty blocks all web access.",
                    required: false,
                },
                FieldSchema {
                    name: "allow_all",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "\"Allow all sites\" toggle. true sets the allowlist to [\"*\"]; false drops the wildcard, keeping explicit hosts.",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_search_settings" => ControllerSchema {
            namespace: "config",
            function: "get_search_settings",
            description:
                "Read search engine settings. API keys are surfaced as presence booleans only.",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "Engine, effective engine, limits, and per-provider configuration flags.",
            )],
        },
        "get_activity_level_settings" => ControllerSchema {
            namespace: "config",
            function: "get_activity_level_settings",
            description: "Get the agent activity level (0–4) and its derived settings: sync cadence, heartbeat/subconscious toggles, token budget, estimated monthly cost.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Activity level settings with cost estimates.")],
        },
        "update_activity_level_settings" => ControllerSchema {
            namespace: "config",
            function: "update_activity_level_settings",
            description: "Set the agent activity level. Immediately updates the scheduler gate mode and persists the change.",
            inputs: vec![optional_string("level", "Activity level: off | minimal | moderate | active | always_on (or 0–4).")],
            outputs: vec![json_output("settings", "Updated activity level settings with cost estimates.")],
        },
        "get_memory_sync_settings" => ControllerSchema {
            namespace: "config",
            function: "get_memory_sync_settings",
            description: "Get the global memory-sync cadence applied to all opted-in sources: stored value, resolved selected cadence, manual/default flags, the 24h default, and the preset options (4h/12h/24h).",
            inputs: vec![],
            outputs: vec![json_output("settings", "Memory sync schedule settings.")],
        },
        "update_memory_sync_settings" => ControllerSchema {
            namespace: "config",
            function: "update_memory_sync_settings",
            description: "Set the global memory-sync cadence. Omit/null resets to the default; 0 means Manual only (auto-sync disabled); a positive value is seconds between syncs. Takes effect on the next scheduler tick.",
            inputs: vec![FieldSchema {
                name: "sync_interval_secs",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Seconds between auto-syncs. null = default (24h); 0 = Manual only; n>0 = sync every n seconds.",
                required: false,
            }],
            outputs: vec![json_output("settings", "Updated memory sync schedule settings.")],
        },
        "get_sandbox_settings" => ControllerSchema {
            namespace: "config",
            function: "get_sandbox_settings",
            description: "Get sandbox execution backend settings: selected backend, Docker image/limits, env passthrough, Docker availability, and detected OS backend.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Sandbox settings with status.")],
        },
        "update_sandbox_settings" => ControllerSchema {
            namespace: "config",
            function: "update_sandbox_settings",
            description: "Update sandbox execution backend settings: backend selection, Docker image, memory/CPU limits, and env passthrough. Applies to new agent sessions.",
            inputs: vec![
                optional_string("backend", "Sandbox backend: auto | landlock | firejail | bubblewrap | docker | none."),
                optional_bool("enabled", "Enable or disable sandbox execution."),
                optional_string("docker_image", "Docker image for sandboxed execution (e.g. alpine:3.20)."),
                FieldSchema {
                    name: "docker_memory_limit_mb",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Docker container memory limit in MB.",
                    required: false,
                },
                FieldSchema {
                    name: "docker_cpu_limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Docker container CPU limit (e.g. 1.0 = one core).",
                    required: false,
                },
                FieldSchema {
                    name: "env_passthrough",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                    comment: "Environment variables to pass through into the sandbox.",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "agent_server_status" => ControllerSchema {
            namespace: "config",
            function: "agent_server_status",
            description: "Return agent server runtime URL and status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Agent server status payload.")],
        },
        "reset_local_data" => ControllerSchema {
            namespace: "config",
            function: "reset_local_data",
            description:
                "Delete local OpenHuman data for the active config/workspace so the next restart boots clean.",
            inputs: vec![],
            outputs: vec![json_output("result", "Reset result with removed paths.")],
        },
        "get_data_paths" => ControllerSchema {
            namespace: "config",
            function: "get_data_paths",
            description:
                "Resolve the OpenHuman data directories (current workspace, default ~/.openhuman, active workspace marker) that reset_local_data would remove. Read-only — performs no filesystem changes.",
            inputs: vec![],
            outputs: vec![json_output(
                "paths",
                "Resolved data paths: current_openhuman_dir, default_openhuman_dir, active_workspace_marker_path.",
            )],
        },
        "get_agent_paths" => ControllerSchema {
            namespace: "config",
            function: "get_agent_paths",
            description:
                "Resolve the agent's filesystem roots (action_dir, workspace_dir, projects_dir) so the UI can render live values instead of hard-coded strings. Read-only. Also returns `action_dir_env_override: bool` so the UI knows when OPENHUMAN_ACTION_DIR is forcing the value (Settings → action_dir editing disabled in that case).",
            inputs: vec![],
            outputs: vec![json_output(
                "paths",
                "Resolved agent paths: action_dir (acting-tool CWD), workspace_dir (internal state, agent-blocked), projects_dir (default projects home), action_dir_source (env | override | default).",
            )],
        },
        "update_agent_paths" => ControllerSchema {
            namespace: "config",
            function: "update_agent_paths",
            description:
                "Update the agent's editable filesystem roots. Currently only action_dir (the acting-tool sandbox). The path must be absolute; a missing directory is auto-created; it cannot equal the internal workspace_dir. An empty string clears the override and reverts to the default. Applies to new sessions immediately (live policy hot-swap), no restart. OPENHUMAN_ACTION_DIR still overrides at runtime when set.",
            inputs: vec![FieldSchema {
                name: "action_dir",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "New absolute action sandbox path. Empty string clears the override (revert to default). Omit to leave unchanged.",
                required: false,
            }],
            outputs: vec![json_output(
                "paths",
                "Updated agent paths (same shape as get_agent_paths): action_dir, workspace_dir, projects_dir, action_dir_source.",
            )],
        },
        "get_onboarding_completed" => ControllerSchema {
            namespace: "config",
            function: "get_onboarding_completed",
            description: "Read whether the user has completed the onboarding flow.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "completed",
                ty: TypeSchema::Bool,
                comment: "True when onboarding has been completed.",
                required: true,
            }],
        },
        "get_dictation_settings" => ControllerSchema {
            namespace: "config",
            function: "get_dictation_settings",
            description: "Read current voice dictation settings.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Dictation settings payload.")],
        },
        "update_dictation_settings" => ControllerSchema {
            namespace: "config",
            function: "update_dictation_settings",
            description: "Update voice dictation settings.",
            inputs: vec![
                optional_bool("enabled", "Enable voice dictation."),
                optional_string("hotkey", "Global hotkey string (e.g. Fn)."),
                optional_string("activation_mode", "Activation mode: toggle or push."),
                optional_bool("llm_refinement", "Enable LLM post-processing of transcription."),
                optional_bool("streaming", "Enable WebSocket streaming transcription."),
                FieldSchema {
                    name: "streaming_interval_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Interval between streaming inference passes (ms).",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_voice_server_settings" => ControllerSchema {
            namespace: "config",
            function: "get_voice_server_settings",
            description: "Read current voice server settings.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Voice server settings payload.")],
        },
        "update_voice_server_settings" => ControllerSchema {
            namespace: "config",
            function: "update_voice_server_settings",
            description: "Update voice server settings.",
            inputs: vec![
                optional_bool("auto_start", "Start the voice server automatically with the core."),
                optional_string("hotkey", "Voice server hotkey string (e.g. Fn)."),
                optional_string("activation_mode", "Activation mode: tap or push."),
                optional_bool("skip_cleanup", "Skip LLM cleanup and keep dictation verbatim."),
                FieldSchema {
                    name: "min_duration_secs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "Minimum recording duration in seconds.",
                    required: false,
                },
                FieldSchema {
                    name: "silence_threshold",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "RMS energy threshold for silence detection.",
                    required: false,
                },
                FieldSchema {
                    name: "custom_dictionary",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Custom vocabulary words to bias whisper toward.",
                    required: false,
                },
                optional_bool(
                    "always_on_enabled",
                    "Continuous always-on listening (no hotkey). Opt-in.",
                ),
                optional_string(
                    "wake_word",
                    "Always-on wake word; utterances must contain it (default 'Hey Tiny').",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "set_onboarding_completed" => ControllerSchema {
            namespace: "config",
            function: "set_onboarding_completed",
            description: "Mark the onboarding flow as completed or reset it.",
            inputs: vec![FieldSchema {
                name: "value",
                ty: TypeSchema::Bool,
                comment: "True to mark completed, false to reset.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "completed",
                ty: TypeSchema::Bool,
                comment: "Updated onboarding completed state.",
                required: true,
            }],
        },
        "update_composio_trigger_settings" => ControllerSchema {
            namespace: "config",
            function: "update_composio_trigger_settings",
            description:
                "Update Composio trigger-triage settings. When triage is disabled the \
                 local LLM is NOT invoked per trigger — events are still archived to \
                 trigger history.",
            inputs: vec![
                optional_bool(
                    "triage_disabled",
                    "When true, skip the LLM triage turn for all Composio triggers globally.",
                ),
                FieldSchema {
                    name: "triage_disabled_toolkits",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Toolkit slugs that skip LLM triage (e.g. [\"gmail\", \"slack\"]).",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        },
        "get_composio_trigger_settings" => ControllerSchema {
            namespace: "config",
            function: "get_composio_trigger_settings",
            description: "Read current Composio trigger-triage settings.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "triage_disabled",
                    ty: TypeSchema::Bool,
                    comment: "Whether the global triage-disabled flag is set.",
                    required: true,
                },
                FieldSchema {
                    name: "triage_disabled_toolkits",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Toolkit slugs that skip LLM triage.",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "config",
            function: "unknown",
            description: "Unknown config controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}
