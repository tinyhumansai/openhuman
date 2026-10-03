//! Schemas for agent behaviour settings: autonomy, privacy, browser, sandbox, and memory sync.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::super::helpers::{json_output, optional_bool, optional_number, optional_string};

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
            description: "Update the agent access mode: whether the policy is enabled at all, autonomy level, workspace confinement, trusted-roots allow-list, command allow-list, forbidden paths, and OS-install permission. Applies live to active sessions.",
            inputs: vec![
                optional_bool("enabled", "Master switch for the autonomy policy. Defaults to false: with it off, command classification, the approval gate, the command allow-list, the action budget and workspace containment are all inert, and every other field here has no effect. Credential stores and system roots stay blocked either way."),
                optional_string("level", "Autonomy level: readonly | supervised | full. Only binds when enabled is true."),
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
                    // A non-zero `u32`: 0 is refused by the apply path, and
                    // u32::MAX is the "unlimited" sentinel the UI saves.
                    ty: TypeSchema::Option(Box::new(TypeSchema::BoundedU64 {
                        min: 1,
                        max: u32::MAX as u64,
                    })),
                    comment: "Rate limit for side-effecting actions per hour (1..=4294967295; 4294967295 = unlimited).",
                    required: false,
                },
                FieldSchema {
                    name: "auto_approve",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                    comment: "Replace the \"Always allow\" allowlist (array of tool names the agent runs without an approval prompt). Empty array clears it.",
                    required: false,
                },
                optional_bool("auto_approve_all", "When true, auto-approve all tool calls without prompting. Unknown origins still denied. Hard security blocks unaffected."),
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
            description: "Read agent execution settings: the action/tool wall-clock timeout, the runtime-effective value, and whether the OPENHUMAN_TOOL_TIMEOUT_SECS env var overrides it.",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "Agent settings: agent_timeout_secs, effective_timeout_secs, env_override, min_timeout_secs, max_timeout_secs, tool_dispatcher, tool_dispatcher_env_override.",
            )],
        }),
"update_agent_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_agent_settings",
            description: "Update agent execution settings: the action/tool wall-clock timeout (seconds) and the web-chat target agent. Applies to the next tool call without a restart; the OPENHUMAN_TOOL_TIMEOUT_SECS env var still overrides it when set.",
            inputs: vec![FieldSchema {
                name: "agent_timeout_secs",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Wall-clock timeout for a single tool/action execution, in seconds (1–3600). Extend this when large local models are interrupted before finishing.",
                required: false,
            },
            FieldSchema {
                name: "chat_agent_id",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Agent definition id the web-chat path routes turns to. Empty string reverts to the orchestrator. A named definition's own max_iterations governs the turn, so this is how a longer-running agent is selected.",
                required: false,
            },
            FieldSchema {
                name: "tool_dispatcher",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "How tool calls are spoken to the model: auto (default; native when supported, else JSON-in-tag) | native | xml | pformat | python | typescript. Applies to new sessions.",
                required: false,
            }],
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
                    "Browser backend: tinycomputer (legacy values, including tinybrowser, accepted for migration).",
                ),
                optional_bool("headless", "Run Chrome without a visible window."),
                optional_number("viewport_width", "Chrome viewport width in pixels (320-3840)."),
                optional_number("viewport_height", "Chrome viewport height in pixels (240-2160)."),
                optional_string("chrome_path", "Optional Chrome executable path; empty clears."),
                optional_string("profile_mode", "fresh or persistent."),
                optional_string("profile_path", "Persistent Chrome profile path; empty clears."),
                optional_string("download_dir", "Absolute permitted download folder; empty clears."),
                optional_number("max_task_steps", "Maximum Jev task steps (1-100)."),
                optional_number("task_timeout_secs", "Browser task timeout in seconds (5-600)."),
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
"update_computer_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_computer_settings",
            description: "Update TinyComputer's decision, planner and rescue models.",
            inputs: vec![
                optional_string("decision_model", "Decision model: jev, open_jev, or sage."),
                optional_bool("sage_fast", "Use Sage's fast mode."),
                optional_string("planner_model", "Planner model id; empty restores the module default."),
                optional_string("rescue_model", "Rescue model id for failed steps; empty restores the module default."),
                optional_number("max_rescues", "Rescues allowed per task (0-5); 0 turns rescue off."),
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
