//! Tool: ax_interact — interact with desktop app UI via the OS accessibility API.
//!
//! Cross-platform: macOS uses AXUIElement (Swift helper), Windows uses UI
//! Automation (UIA COM API). Both back-ends:
//!   - Never crash CEF (no synthetic key/mouse events injected system-wide)
//!   - Work regardless of which app is focused
//!   - Find elements by semantic label, not pixel coordinates
//!
//! Three actions:
//!   list       — enumerate interactive elements in a running app
//!   press      — activate a button/control by label
//!   set_value  — type text into a field by label
//!
//! Requires: macOS Accessibility permission granted to OpenHuman. On Windows no
//! special permission is needed for same-integrity-level apps (UIPI blocks
//! driving an elevated app from a non-elevated process).

use crate::openhuman::accessibility::ax_interact as ax;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Apps whose UI must never be actuated by the agent. `press` / `set_value`
/// are refused when `app_name` matches any of these (case-insensitive
/// substring) — defense-in-depth that holds even on background/auto-approved
/// turns where the ApprovalGate may not prompt. `list` is also refused so the
/// agent can't enumerate, e.g., a password manager's fields. Matched by display
/// name; broad substrings ("keychain", "1password") cover localized variants.
const SENSITIVE_APPS: &[&str] = &[
    "keychain",
    "1password",
    "bitwarden",
    "lastpass",
    "dashlane",
    "system settings",
    "system preferences",
    "console", // macOS Console (logs)
    // Terminal emulators — mirror the set helper.rs treats as terminals
    // (helper.rs ~:557) so "terminals are denied" actually holds.
    "terminal",
    "iterm",
    "wezterm",
    "warp",
    "alacritty",
    "kitty",
    "ghostty",
    "hyper",
    "rio",
];

/// True when `app_name` is on the never-actuate denylist. `pub(crate)` so the
/// `automate` tool shares the exact same boundary as `ax_interact`.
pub(crate) fn is_sensitive_app(app_name: &str) -> bool {
    let lower = app_name.to_lowercase();
    SENSITIVE_APPS.iter().any(|s| lower.contains(s))
}

pub struct AxInteractTool {
    /// When false, the mutating actions (`press` / `set_value`) are refused
    /// with guidance to enable `computer_control.ax_interact_mutations`. The
    /// read-only `list` action is always available. Like the mouse/keyboard
    /// tools (`computer_control.enabled`), this is opt-in **and** approval-gated:
    /// the mutating actions return `external_effect_with_args == true` so they
    /// route through the ApprovalGate.
    allow_mutations: bool,
}

impl AxInteractTool {
    pub fn new(allow_mutations: bool) -> Self {
        Self { allow_mutations }
    }
}

impl Default for AxInteractTool {
    fn default() -> Self {
        // Default to read-only (mutations opt-in) — safe baseline.
        Self::new(false)
    }
}

#[async_trait]
impl Tool for AxInteractTool {
    fn name(&self) -> &str {
        "ax_interact"
    }

    fn description(&self) -> &str {
        "Interact with ANY desktop application's UI using the platform accessibility API \
         (macOS AXUIElement / Windows UI Automation). Finds buttons, text fields, list rows, \
         and controls by their label — no screen coordinates, no synthetic key/mouse events. \
         Works for any app: a music player, browser, mail, notes, Slack, system settings, etc.\n\
         \n\
         Actions:\n\
         • 'list' → show interactive elements. ALWAYS pass a `filter` substring to narrow \
         results (apps expose hundreds of elements; an unfiltered list is huge and unreliable). \
         e.g. filter='Play', filter='Send', filter='Highway'.\n\
         • 'press' → activate a button/control/row by label (exact match preferred). \
         e.g. label='Play', label='Send', label='OK'.\n\
         • 'set_value' → type text into a field by label (omit label for the first text field).\n\
         \n\
         General pattern: (1) `list` with a `filter` to find the element, (2) `press` it. \
         Note that in many apps, pressing a LIST ROW or SEARCH RESULT only selects/opens it — \
         to trigger an action you then press the relevant action button (e.g. after opening a \
         song's page, press its 'Play' button). If a press doesn't have the intended effect, \
         `list` again to see the new screen and press the actual action control.\n\
         \n\
         On macOS this requires Accessibility permission for OpenHuman; on Windows no special \
         permission is needed for normal (non-elevated) apps."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "press", "set_value"],
                    "description": "'list' = show interactive elements (use with filter); 'press' = activate a control by label; 'set_value' = type into a text field."
                },
                "app_name": {
                    "type": "string",
                    "description": "Display name of the running application (e.g. 'Music', 'Safari', 'Telegram')."
                },
                "filter": {
                    "type": "string",
                    "description": "For 'list': only return elements whose label contains this substring (case-insensitive). Strongly recommended — keeps results small and accurate."
                },
                "label": {
                    "type": "string",
                    "description": "For 'press'/'set_value': label of the element to target (case-insensitive, exact match preferred). For 'set_value', omit to target the first text field."
                },
                "value": {
                    "type": "string",
                    "description": "Text to enter (required for 'set_value')."
                }
            },
            "required": ["action", "app_name"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Minimum across actions: `list` is read-only. The per-call level
        // (Dangerous for press/set_value) is enforced by
        // `permission_level_with_args`. Returning the minimum here keeps the
        // tool available on channels that can run the read-only `list`.
        PermissionLevel::ReadOnly
    }

    fn permission_level_with_args(&self, args: &serde_json::Value) -> PermissionLevel {
        match args.get("action").and_then(|v| v.as_str()) {
            // `list` only reads the AX tree — no state change.
            Some("list") | None => PermissionLevel::ReadOnly,
            // `press` / `set_value` actuate real controls (click buttons,
            // type into fields) and change application state, so they must
            // not ride on the read-only path.
            _ => PermissionLevel::Dangerous,
        }
    }

    fn external_effect_with_args(&self, args: &serde_json::Value) -> bool {
        // Route mutating actions through the ApprovalGate before execute();
        // `list` is a pure read and flows through unprompted.
        !matches!(
            args.get("action").and_then(|v| v.as_str()),
            Some("list") | None
        )
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
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let app_name = args
            .get("app_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let label = args
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let value = args
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let filter = args
            .get("filter")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        log::info!(
            "[ax_interact] ▶ action={action:?} app={app_name:?} label={label:?} filter={filter:?}"
        );

        if app_name.is_empty() {
            return Ok(ToolResult::error("app_name is required"));
        }

        let mutating = matches!(action.as_str(), "press" | "set_value");

        // Denylist: never actuate or enumerate sensitive apps (password
        // managers, Keychain, System Settings, terminals). Defense-in-depth
        // that holds even when the ApprovalGate doesn't prompt (background /
        // auto-approved turns).
        if is_sensitive_app(&app_name) {
            log::warn!("[ax_interact] refused: sensitive app '{app_name}' (action={action})");
            return Ok(ToolResult::error(format!(
                "Refusing to interact with '{app_name}': it is on the sensitive-app denylist \
                 (password managers, Keychain, System Settings, terminals). This is a hard \
                 safety boundary."
            )));
        }

        // Mutating actions are opt-in. Read-only `list` is always allowed.
        if mutating && !self.allow_mutations {
            log::warn!("[ax_interact] refused: mutations disabled (action={action})");
            return Ok(ToolResult::error(
                "App control isn't enabled yet, so I can't press buttons or type into \
                 this app. Turn on App UI Control / App Automation in Settings → Agent \
                 Access, then ask again. (Reading the UI still works without it; sets \
                 computer_control.ax_interact_mutations = true.)",
            ));
        }

        // Cap how many elements we render so a broad/empty filter can't overflow
        // the tool-result budget and cause the model to reason over a truncated view.
        const MAX_LISTED: usize = 60;

        let result = match action.as_str() {
            "list" => match ax::ax_list_elements_filtered(&app_name, &filter) {
                Ok(elements) if elements.is_empty() => {
                    log::info!(
                        "[ax_interact] list: no elements in '{app_name}' (filter={filter:?})"
                    );
                    let hint = if filter.is_empty() {
                        format!(
                            "No interactive elements found in '{app_name}'. \
                             The app may not expose its UI tree via Accessibility API, \
                             or OpenHuman may need Accessibility permission."
                        )
                    } else {
                        format!(
                            "No elements in '{app_name}' match filter '{filter}'. \
                             The UI may still be loading — wait and try again, or call \
                             'list' with a shorter/different filter."
                        )
                    };
                    ToolResult::success(hint)
                }
                Ok(elements) => {
                    let total = elements.len();
                    log::info!(
                        "[ax_interact] list: {total} elements in '{app_name}' (filter={filter:?})"
                    );
                    let shown = total.min(MAX_LISTED);
                    let lines: Vec<String> = elements
                        .iter()
                        .take(MAX_LISTED)
                        .map(|e| format!("  [{role}] {label}", role = e.role, label = e.label))
                        .collect();
                    let mut out = if filter.is_empty() {
                        format!("Elements in '{app_name}' (showing {shown} of {total}):\n")
                    } else {
                        format!(
                            "Elements in '{app_name}' matching '{filter}' (showing {shown} of {total}):\n"
                        )
                    };
                    out.push_str(&lines.join("\n"));
                    if total > MAX_LISTED {
                        out.push_str(&format!(
                            "\n… {} more — narrow with a more specific `filter`.",
                            total - MAX_LISTED
                        ));
                    }
                    ToolResult::success(out)
                }
                Err(e) => {
                    log::warn!("[ax_interact] list failed: {e}");
                    ToolResult::error(e)
                }
            },

            "press" => {
                if label.is_empty() {
                    return Ok(ToolResult::error(
                        "'label' is required for action='press'. Use action='list' first to discover element labels.",
                    ));
                }
                match ax::ax_press_element(&app_name, &label) {
                    Ok(msg) => {
                        log::info!("[ax_interact] press succeeded: {msg}");
                        ToolResult::success(msg)
                    }
                    Err(e) => {
                        log::warn!("[ax_interact] press failed: {e}");
                        ToolResult::error(format!(
                            "{e}. Try action='list' to see available element labels."
                        ))
                    }
                }
            }

            "set_value" => {
                if value.is_empty() {
                    return Ok(ToolResult::error(
                        "'value' is required for action='set_value'",
                    ));
                }
                match ax::ax_set_field_value(&app_name, &label, &value) {
                    Ok(msg) => {
                        log::info!("[ax_interact] set_value succeeded: {msg}");
                        ToolResult::success(msg)
                    }
                    Err(e) => {
                        log::warn!("[ax_interact] set_value failed: {e}");
                        ToolResult::error(format!(
                            "{e}. Try action='list' to see available text field labels."
                        ))
                    }
                }
            }

            other => ToolResult::error(format!(
                "Unknown action '{other}'. Valid actions: 'list', 'press', 'set_value'."
            )),
        };

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_permission() {
        let tool = AxInteractTool::new(true);
        assert_eq!(tool.name(), "ax_interact");
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
        // Mutating actions gate per-call.
        assert_eq!(
            tool.permission_level_with_args(&json!({"action": "press"})),
            PermissionLevel::Dangerous
        );
        assert!(tool.external_effect_with_args(&json!({"action": "press"})));
        assert!(!tool.external_effect_with_args(&json!({"action": "list"})));
    }

    #[test]
    fn schema_requires_action_and_app_name() {
        let schema = AxInteractTool::new(true).parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "action"));
        assert!(required.iter().any(|v| v == "app_name"));
    }

    #[test]
    fn sensitive_apps_detected() {
        assert!(is_sensitive_app("Keychain Access"));
        assert!(is_sensitive_app("1Password 7"));
        assert!(is_sensitive_app("System Settings"));
        assert!(is_sensitive_app("Terminal"));
        // All terminal emulators helper.rs recognizes must also be denied.
        for t in [
            "iTerm",
            "WezTerm",
            "Warp",
            "Alacritty",
            "kitty",
            "Ghostty",
            "Hyper",
            "Rio",
        ] {
            assert!(is_sensitive_app(t), "expected '{t}' to be denied");
        }
        assert!(!is_sensitive_app("Music"));
        assert!(!is_sensitive_app("Safari"));
    }

    #[tokio::test]
    async fn rejects_missing_app_name() {
        let result = AxInteractTool::new(true)
            .execute(json!({"action": "list", "app_name": ""}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn rejects_press_without_label() {
        let result = AxInteractTool::new(true)
            .execute(json!({"action": "press", "app_name": "Music"}))
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn refuses_mutations_when_disabled() {
        // mutations off → press/set_value blocked, but list still allowed past this guard.
        let tool = AxInteractTool::new(false);
        let press = tool
            .execute(json!({"action": "press", "app_name": "Music", "label": "Play"}))
            .await
            .unwrap();
        assert!(press.is_error);
        assert!(press.output().contains("ax_interact_mutations"));
    }

    #[tokio::test]
    async fn refuses_sensitive_app_even_with_mutations() {
        let tool = AxInteractTool::new(true);
        for app in [
            "Keychain Access",
            "1Password",
            "Terminal",
            "System Settings",
        ] {
            let r = tool
                .execute(json!({"action": "press", "app_name": app, "label": "OK"}))
                .await
                .unwrap();
            assert!(r.is_error, "expected refusal for {app}");
            assert!(r.output().to_lowercase().contains("denylist"));
        }
    }
}
