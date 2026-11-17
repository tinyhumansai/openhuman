//! Slash command effects.
use super::*;

pub(super) async fn execute_command(
    command_line: &str,
    runtime: &Arc<CoreRuntime>,
    client_id: &str,
    state: &mut TranscriptState,
    ui: &mut UiState,
) -> bool {
    let mut parts = command_line.split_whitespace();
    let command = parts.next().unwrap_or("help");
    let argument = parts.collect::<Vec<_>>().join(" ");
    if crate::presentation::open(command, state, ui) {
        return false;
    }
    match command {
        "quit" => return true,
        "new" => new_thread(runtime, state, ui).await,
        "help" => {
            let mut overlay = Overlay::new(OverlayKind::Help, "Agent cockpit help");
            overlay.status = "Type to filter · Esc closes".into();
            overlay.rows = crate::composer::COMMANDS
                .iter()
                .map(|(name, description)| OverlayRow {
                    id: (*name).into(),
                    label: format!("/{name}"),
                    detail: (*description).into(),
                    payload: serde_json::Value::Null,
                })
                .collect();
            ui.overlay = Some(overlay);
        }
        "resume" | "sessions" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Threads,
                "Saved threads",
                "openhuman.threads_list",
                json!({}),
                &["threads", "items"],
                &["id", "thread_id"],
                &["title", "name", "id"],
            )
            .await;
        }
        "rename" => {
            let mut overlay = Overlay::new(OverlayKind::Rename, "Rename thread");
            overlay.input = Some(argument);
            overlay.status = "Enter saves · Esc cancels".into();
            ui.overlay = Some(overlay);
        }
        "delete" => {
            let mut overlay = Overlay::new(OverlayKind::ConfirmDelete, "Delete thread?");
            overlay.status = "Press y to permanently delete this conversation, or Esc.".into();
            ui.overlay = Some(overlay);
        }
        "model" | "models" => {
            if argument.is_empty() {
                open_rpc_overlay(
                    runtime,
                    ui,
                    OverlayKind::Model,
                    "Available models",
                    "openhuman.inference_list_models",
                    json!({"provider_id":ui.provider_id}),
                    &["models"],
                    &["id"],
                    &["name", "id"],
                )
                .await;
                if let Some(overlay) = &mut ui.overlay {
                    overlay.rows.insert(
                        0,
                        text_row("", "Configured default", "Use the configured model"),
                    );
                    overlay.status = "Enter selects · /model <id> sets an explicit override".into();
                }
                return false;
            }
            let mut overlay = Overlay::new(OverlayKind::Model, "Model override");
            overlay.input = Some(argument);
            overlay.status = "Enter applies a model id to subsequent turns · empty clears".into();
            ui.overlay = Some(overlay);
        }
        "permissions" => {
            let mut overlay = Overlay::new(OverlayKind::Permissions, "Agent access");
            overlay.status = if ui.policy_enabled {
                "Choose an access tier · Enter applies".into()
            } else {
                "Policy is OFF · stored tiers take effect only when core autonomy policy is enabled"
                    .into()
            };
            overlay.rows = [
                (
                    "readonly",
                    "Read-only",
                    "With policy enabled: writes, network, and installs are blocked",
                ),
                ("supervised", "Supervised", "Risky actions ask for approval"),
                (
                    "full",
                    "Full access",
                    "Allowed actions run without prompting",
                ),
            ]
            .into_iter()
            .map(|(id, label, detail)| OverlayRow {
                id: id.into(),
                label: label.into(),
                detail: detail.into(),
                payload: serde_json::Value::Null,
            })
            .collect();
            ui.overlay = Some(overlay);
        }
        "status" => {
            let mut overlay = Overlay::new(OverlayKind::Status, "Session status");
            refresh_agent_paths(runtime, ui).await;
            if let Ok(value) = runtime
                .invoke(
                    "openhuman.channel_web_queue_status",
                    json!({"thread_id": ui.thread_id}),
                )
                .await
            {
                ui.queue_status = serde_json::to_string(crate::cockpit::unwrap_rpc(&value))
                    .unwrap_or_else(|_| "unavailable".into());
            }
            overlay.rows = vec![
                text_row("thread", "Thread", &ui.thread_id),
                text_row(
                    "model",
                    "Model",
                    ui.model_override.as_deref().unwrap_or("configured default"),
                ),
                text_row(
                    "cwd",
                    "Action directory",
                    if ui.action_dir.is_empty() {
                        "unavailable"
                    } else {
                        &ui.action_dir
                    },
                ),
                text_row(
                    "queue",
                    "Run queue",
                    if ui.queue_status.is_empty() {
                        "idle"
                    } else {
                        &ui.queue_status
                    },
                ),
            ];
            ui.overlay = Some(overlay);
        }
        "usage" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Usage,
                "Token and cost usage",
                "openhuman.threads_token_usage",
                json!({"thread_id": ui.thread_id}),
                &[],
                &[],
                &[],
            )
            .await
        }
        "agents" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Agents,
                "Agents",
                "openhuman.agent_list_definitions",
                json!({}),
                &["definitions", "items"],
                &["id"],
                &["display_name", "name", "id"],
            )
            .await
        }
        "skills" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Skills,
                "Skills",
                "openhuman.skills_list",
                json!({"include_skills": true}),
                &["skills", "items"],
                &["id"],
                &["name", "title", "id"],
            )
            .await
        }
        "mcp" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Mcp,
                "MCP servers",
                "openhuman.mcp_clients_installed_list",
                json!({}),
                &["installed", "servers"],
                &["id", "server_id"],
                &["name", "display_name", "id"],
            )
            .await
        }
        "artifacts" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Artifacts,
                "Artifacts",
                "openhuman.ai_list_artifacts",
                json!({"thread_id": ui.thread_id, "limit": 100}),
                &["artifacts", "items"],
                &["id", "artifact_id"],
                &["title", "name", "filename", "id"],
            )
            .await
        }
        "approvals" => {
            open_rpc_overlay(
                runtime,
                ui,
                OverlayKind::Approvals,
                "Pending approvals",
                "openhuman.approval_list_pending",
                json!({}),
                &["pending", "approvals"],
                &["request_id", "id"],
                &["tool_name", "message", "request_id"],
            )
            .await
        }
        "diff" => open_git_diff(ui).await,
        "review" => {
            ui.composer.set_text("Review the current Git working tree. Explain correctness risks, regressions, and missing tests with file and line references.");
            send_message(
                runtime,
                client_id,
                state,
                ui,
                if state.is_streaming() {
                    "followup"
                } else {
                    "interrupt"
                },
            );
        }
        "copy" => copy_latest_answer(state),
        "export" => export_transcript(state, ui, &argument),
        "clear" => state.clear(),
        "logs" => ui.active_tab = AppTab::Logs,
        "config" => ui.active_tab = AppTab::Config,
        "settings" => ui.active_tab = AppTab::Settings,
        "login" => {
            if let Some(provider) = crate::account::provider(&argument) {
                crate::account::browser(runtime, ui, provider);
            } else {
                crate::account::picker(ui);
            }
        }
        "login-token" => {
            crate::account::form(ui, false);
        }
        "login-cancel" => crate::account::cancel(ui),
        "login-open" => crate::account::reopen(ui),
        "login-copy" => crate::controls::copy_login_link(ui),
        "plan" => present_pending_plan_review(ui),
        "logout" => {
            crate::account::form(ui, true);
        }
        _ => state.push_system(format!("Unknown command /{command}. Type /help.")),
    }
    false
}
