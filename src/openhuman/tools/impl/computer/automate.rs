//! Tool: `automate` — accomplish a multi-step UI goal in one call.
//!
//! The orchestrator calls `automate{app, goal}` once; the Rust loop in
//! `accessibility::automate` then perceives → decides (fast model) → acts →
//! settles → verifies until the goal is met or a step budget is hit. This keeps
//! the heavy chat model out of the click loop (latency + reliability — see
//! `docs/voice-automate-plan.md`).
//!
//! Safety mirrors `ax_interact`: it actuates real controls, so it is a mutating
//! tool — opt-in via `computer_control.ax_interact_mutations`, routed through the
//! ApprovalGate, and it refuses the sensitive-app denylist (password managers,
//! Keychain, System Settings, terminals) even on auto-approved turns.

use super::ax_interact::is_sensitive_app;
use crate::openhuman::accessibility::automate::{self, AutomateOptions, RealBackend};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct AutomateTool {
    /// When false the tool refuses to run (it is inherently mutating). Mirrors
    /// `AxInteractTool::allow_mutations` so one opt-in governs both.
    allow_mutations: bool,
}

impl AutomateTool {
    pub fn new(allow_mutations: bool) -> Self {
        Self { allow_mutations }
    }
}

impl Default for AutomateTool {
    fn default() -> Self {
        Self::new(false)
    }
}

#[async_trait]
impl Tool for AutomateTool {
    fn name(&self) -> &str {
        "automate"
    }

    fn description(&self) -> &str {
        "Accomplish a MULTI-STEP goal inside a desktop app in a single call — e.g. \
         'play <song> in Music', 'message <person> <text> in Slack'. Give the app \
         name and a plain-English goal; the system drives the app's UI step by step \
         (find elements → press/type → verify) using the platform accessibility API, \
         no screen coordinates. Prefer this over issuing many individual \
         `ax_interact` calls when the task needs several UI steps. The app should \
         usually be launched first (or include 'launch' in the goal). Refuses \
         password managers, Keychain, System Settings, and terminals."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "app": {
                    "type": "string",
                    "description": "Display name of the target application (e.g. 'Music', 'Slack')."
                },
                "goal": {
                    "type": "string",
                    "description": "Plain-English description of the multi-step outcome to achieve."
                }
            },
            "required": ["app", "goal"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Always mutating — it actuates controls. Kept as the base level so the
        // approval gate fires regardless of args.
        PermissionLevel::Dangerous
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let app = args
            .get("app")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let goal = args
            .get("goal")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        log::info!("[automate] ▶ tool execute app={app:?} goal={goal:?}");

        if app.is_empty() {
            return Ok(ToolResult::error("app is required"));
        }
        if goal.is_empty() {
            return Ok(ToolResult::error("goal is required"));
        }

        // Hard safety boundary — identical to ax_interact's denylist.
        if is_sensitive_app(&app) {
            log::warn!("[automate] refused: sensitive app '{app}'");
            return Ok(ToolResult::error(format!(
                "Refusing to automate '{app}': it is on the sensitive-app denylist \
                 (password managers, Keychain, System Settings, terminals). This is a \
                 hard safety boundary."
            )));
        }

        if !self.allow_mutations {
            log::warn!("[automate] refused: mutations disabled");
            return Ok(ToolResult::error(
                "App control isn't enabled yet. Turn on App Automation in \
                 Settings → Agent Access (it grants permission to control apps), \
                 then ask again. (Sets computer_control.ax_interact_mutations = true.)",
            ));
        }

        let config = match crate::openhuman::config::rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("could not load config: {e}"))),
        };

        let backend = RealBackend::new(config);
        let outcome = automate::run(&app, &goal, &backend, AutomateOptions::default()).await;

        let mut body = format!("{}\n\nSteps:", outcome.summary);
        if outcome.steps.is_empty() {
            body.push_str("\n  (no steps executed)");
        } else {
            for s in &outcome.steps {
                body.push_str(&format!("\n  - {s}"));
            }
        }

        if outcome.success {
            Ok(ToolResult::success(body))
        } else {
            Ok(ToolResult::error(body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_permission() {
        let t = AutomateTool::new(true);
        assert_eq!(t.name(), "automate");
        assert_eq!(t.permission_level(), PermissionLevel::Dangerous);
        assert!(t.external_effect());
    }

    #[test]
    fn schema_requires_app_and_goal() {
        let schema = AutomateTool::new(true).parameters_schema();
        let req = schema["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "app"));
        assert!(req.iter().any(|v| v == "goal"));
    }

    #[tokio::test]
    async fn rejects_missing_app_or_goal() {
        let t = AutomateTool::new(true);
        assert!(
            t.execute(json!({"app": "", "goal": "x"}))
                .await
                .unwrap()
                .is_error
        );
        assert!(
            t.execute(json!({"app": "Music", "goal": ""}))
                .await
                .unwrap()
                .is_error
        );
    }

    #[tokio::test]
    async fn refuses_when_mutations_disabled() {
        let t = AutomateTool::new(false);
        let r = t
            .execute(json!({"app": "Music", "goal": "play a song"}))
            .await
            .unwrap();
        assert!(r.is_error);
        assert!(r.output().contains("ax_interact_mutations"));
    }

    #[tokio::test]
    async fn refuses_sensitive_app() {
        let t = AutomateTool::new(true);
        for app in [
            "Keychain Access",
            "1Password",
            "Terminal",
            "System Settings",
        ] {
            let r = t
                .execute(json!({"app": app, "goal": "do something"}))
                .await
                .unwrap();
            assert!(r.is_error, "expected refusal for {app}");
            assert!(r.output().to_lowercase().contains("denylist"));
        }
    }
}
