//! Schemas for workspace state: onboarding flags, analytics, dashboard, data and agent paths, and local-data reset.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::super::helpers::{json_output, optional_bool, optional_string};

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
        "workspace_onboarding_flag_exists" => Some(ControllerSchema {
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
        }),
        "workspace_onboarding_flag_set" => Some(ControllerSchema {
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
        }),
        "update_analytics_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "update_analytics_settings",
            description: "Enable or disable anonymized analytics and error reporting.",
            inputs: vec![optional_bool(
                "enabled",
                "Enable anonymized analytics and crash reports.",
            )],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
        "update_user_timezone" => Some(ControllerSchema {
            namespace: "config",
            function: "update_user_timezone",
            description: "Set the user's IANA time zone; null or blank follows the device.",
            inputs: vec![optional_string(
                "timezone",
                "IANA time zone name, e.g. Asia/Kolkata; null or blank to follow the device.",
            )],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
        "get_user_timezone" => Some(ControllerSchema {
            namespace: "config",
            function: "get_user_timezone",
            description: "Read the user's time zone setting, the device's zone, and the one in effect.",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "{ timezone: string | null, device: string | null, effective: string }.",
            )],
        }),
        "get_analytics_settings" => Some(ControllerSchema {
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
        }),
        "get_dashboard_settings" => Some(ControllerSchema {
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
        }),
        "agent_server_status" => Some(ControllerSchema {
            namespace: "config",
            function: "agent_server_status",
            description: "Return agent server runtime URL and status.",
            inputs: vec![],
            outputs: vec![json_output("status", "Agent server status payload.")],
        }),
        "reset_local_data" => Some(ControllerSchema {
            namespace: "config",
            function: "reset_local_data",
            description: "Delete local OpenHuman data for the active config/workspace so the next restart boots clean.",
            inputs: vec![],
            outputs: vec![json_output("result", "Reset result with removed paths.")],
        }),
        "get_data_paths" => Some(ControllerSchema {
            namespace: "config",
            function: "get_data_paths",
            description: "Resolve the OpenHuman data directories (current workspace, default ~/.openhuman, active workspace marker) that reset_local_data would remove. Read-only — performs no filesystem changes.",
            inputs: vec![optional_string(
                "user_id",
                "Resolve paths for this specific user id (users/<id>) instead of the active-user marker. Clear App Data passes this because it signs the user out — removing the marker — before deleting the data.",
            )],
            outputs: vec![json_output(
                "paths",
                "Resolved data paths: current_openhuman_dir, default_openhuman_dir, active_workspace_marker_path.",
            )],
        }),
        "get_agent_paths" => Some(ControllerSchema {
            namespace: "config",
            function: "get_agent_paths",
            description: "Resolve the agent's filesystem roots (action_dir, workspace_dir, projects_dir) so the UI can render live values instead of hard-coded strings. Read-only. Also returns `action_dir_env_override: bool` so the UI knows when OPENHUMAN_ACTION_DIR is forcing the value (Settings → action_dir editing disabled in that case).",
            inputs: vec![],
            outputs: vec![json_output(
                "paths",
                "Resolved agent paths: action_dir (acting-tool CWD), workspace_dir (internal state, agent-blocked), projects_dir (default projects home), action_dir_source (env | override | default), files_dir (where agent deliverables are written), default_files_dir, files_dir_source (override | default).",
            )],
        }),
        "update_agent_paths" => Some(ControllerSchema {
            namespace: "config",
            function: "update_agent_paths",
            description: "Update the agent's editable filesystem roots: action_dir (the acting-tool sandbox) and files_dir (the folder agent deliverables are written to; new artifacts only, and it may not sit inside the OpenHuman data folder). The path must be absolute; a missing directory is auto-created; it cannot equal the internal workspace_dir. An empty string clears the override and reverts to the default. Applies to new sessions immediately (live policy hot-swap), no restart. OPENHUMAN_ACTION_DIR still overrides at runtime when set.",
            inputs: vec![
                FieldSchema {
                    name: "action_dir",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New absolute action sandbox path. Empty string clears the override (revert to default). Omit to leave unchanged.",
                    required: false,
                },
                FieldSchema {
                    name: "files_dir",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New absolute folder for agent deliverables. Empty string clears the override (revert to ~/OpenHuman/projects/Files). Omit to leave unchanged.",
                    required: false,
                },
            ],
            outputs: vec![json_output(
                "paths",
                "Updated agent paths (same shape as get_agent_paths): action_dir, workspace_dir, projects_dir, action_dir_source, files_dir, default_files_dir, files_dir_source.",
            )],
        }),
        "get_onboarding_completed" => Some(ControllerSchema {
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
        }),
        "set_onboarding_completed" => Some(ControllerSchema {
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
        }),
        _ => None,
    }
}
