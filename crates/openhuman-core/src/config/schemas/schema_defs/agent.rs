//! Schemas for agent behaviour settings: autonomy, privacy, browser, sandbox, activity level, and memory sync.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::super::helpers::{json_output, optional_bool, optional_string};

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
"get_autonomy_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "get_autonomy_settings",
            description: "Get the agent access-mode settings (autonomy level, workspace confinement, trusted roots, command allow-list, forbidden paths).",
            inputs: vec![],
            outputs: vec![json_output("autonomy", "Current [autonomy] config block.")],
        }),
"update_autonomy_settings" => Some( ControllerSchema {
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
                optional_bool("auto_approve_all", "When true, auto-approve all tool calls without prompting. SubconsciousTainted and Unknown origins still denied. Hard security blocks unaffected."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
"get_privacy_mode" => Some( ControllerSchema {
            namespace: "config",
            function: "get_privacy_mode",
            description: "Get the active Privacy Mode (data-egress posture): local_only | standard | sensitive. Distinct from the autonomy access mode.",
            inputs: vec![],
            outputs: vec![json_output("mode", "Current privacy mode: local_only | standard | sensitive.")],
        }),
"set_privacy_mode" => Some( ControllerSchema {
            namespace: "config",
            function: "set_privacy_mode",
            description: "Set the Privacy Mode (data-egress posture). local_only blocks external model calls at the inference chokepoint. Applies live to active sessions without a restart.",
            inputs: vec![
                optional_string("mode", "Privacy mode: local_only | standard | sensitive."),
            ],
            outputs: vec![json_output("mode", "Updated privacy mode.")],
        }),
"get_agent_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "get_agent_settings",
            description: "Read agent execution settings: the action/tool wall-clock timeout, the runtime-effective value, whether the OPENHUMAN_TOOL_TIMEOUT_SECS env var overrides it, and allow_metered_agent_tools (persisted user opt-in for TinyHumans/OpenHuman-managed metered tools; sign-in alone does not authorize spending).",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "Agent settings: agent_timeout_secs, allow_metered_agent_tools, effective_timeout_secs, env_override, min_timeout_secs, max_timeout_secs.",
            )],
        }),
"update_agent_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_agent_settings",
            description: "Update agent execution settings. Currently the action/tool wall-clock timeout (seconds). Applies to the next tool call without a restart; the OPENHUMAN_TOOL_TIMEOUT_SECS env var still overrides it when set.",
            inputs: vec![
                FieldSchema {
                    name: "agent_timeout_secs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Wall-clock timeout for a single tool/action execution, in seconds (1–3600). Extend this when large local models are interrupted before finishing.",
                    required: false,
                },
                optional_bool(
                    "allow_metered_agent_tools",
                    "Persisted spending authorization for managed metered agent tools (opt-in). Sign-in does not grant it, and omission leaves the saved value unchanged.",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
"update_browser_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_browser_settings",
            description: "Update browser automation settings.",
            inputs: vec![
                optional_bool("enabled", "Enable browser integration."),
                optional_string(
                    "backend",
                    "Browser backend: agent_browser, playwright, rust_native, computer_use, or auto.",
                ),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
"set_browser_allow_all" => Some( ControllerSchema {
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
        }),
"get_activity_level_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "get_activity_level_settings",
            description: "Get the agent activity level (0–4) and its derived settings: sync cadence, heartbeat/subconscious toggles, token budget, estimated monthly cost.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Activity level settings with cost estimates.")],
        }),
"update_activity_level_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_activity_level_settings",
            description: "Set the agent activity level. Immediately updates the scheduler gate mode and persists the change.",
            inputs: vec![optional_string("level", "Activity level: off | minimal | moderate | active | always_on (or 0–4).")],
            outputs: vec![json_output("settings", "Updated activity level settings with cost estimates.")],
        }),
"get_memory_sync_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "get_memory_sync_settings",
            description: "Get the global memory-sync cadence applied to all opted-in sources: stored value, resolved selected cadence, manual/default flags, the 24h default, and the preset options (4h/12h/24h).",
            inputs: vec![],
            outputs: vec![json_output("settings", "Memory sync schedule settings.")],
        }),
"update_memory_sync_settings" => Some( ControllerSchema {
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
        }),
"get_sandbox_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "get_sandbox_settings",
            description: "Get sandbox execution backend settings: selected backend, Docker image/limits, env passthrough, Docker availability, and detected OS backend.",
            inputs: vec![],
            outputs: vec![json_output("settings", "Sandbox settings with status.")],
        }),
"update_sandbox_settings" => Some( ControllerSchema {
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
        }),
        _ => None,
    }
}
