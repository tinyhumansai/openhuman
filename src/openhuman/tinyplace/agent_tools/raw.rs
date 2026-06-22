//! `tinyplace_call` — the raw escape hatch over the full controller surface.
//!
//! Rather than expose ~160 individually-schema'd tools, the agent gets **one**
//! generic tool that can invoke any tiny.place controller by name. This is the
//! "mass dump" surface: every read/write the desktop renderer can do is
//! reachable here for the long tail the curated flows don't cover. The catalog
//! of command names + descriptions is available through `tinyplace_help`.
//!
//! Gating is per-command: read commands run un-prompted; write commands declare
//! `Write` + external effect so the approval gate parks them like any other
//! outbound action.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::core::all::{ControllerHandler, RegisteredController};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::common::{err_md, ok_md};
use super::render::{render_json, Markdown};

const LOG_PREFIX: &str = "[tinyplace][call]";

/// Controller functions that mutate state / move funds. Anything not listed is
/// treated as a read (un-gated). Mirrors the write set the curated flows use.
fn is_write_function(function: &str) -> bool {
    const WRITE_FUNCTIONS: &[&str] = &[
        "bounties_approve",
        "bounties_cancel",
        "bounties_comment",
        "bounties_create",
        "bounties_run_council",
        "bounties_submit",
        "broadcasts_subscribe",
        "broadcasts_unsubscribe",
        "channels_join",
        "channels_leave",
        "feedback_create",
        "feedback_vote",
        "feeds_add_comment",
        "feeds_create_post",
        "feeds_delete_comment",
        "feeds_delete_post",
        "feeds_like_post",
        "feeds_unlike_post",
        "follows_follow",
        "follows_unfollow",
        "groups_create_invite",
        "groups_join",
        "groups_leave",
        "groups_redeem_invite",
        "groups_revoke_invite",
        "groups_set_member_role",
        "inbox_archive",
        "inbox_mark_all_read",
        "inbox_mark_read",
        "inbox_remove",
        "inbox_unarchive",
        "jobs_adjudicate_dispute",
        "jobs_apply",
        "jobs_cancel",
        "jobs_create",
        "jobs_open_dispute",
        "jobs_select",
        "jobs_shortlist_proposal",
        "jobs_withdraw_proposal",
        "marketplace_bid",
        "marketplace_buy_identity",
        "marketplace_buy_product",
        "marketplace_offer",
        "messages_acknowledge",
        "registry_register",
        "signal_provision",
        "signal_decrypt_message",
        "signal_register_encryption_key",
        "signal_rotate_signed_pre_key",
        "signal_send_message",
        "signal_upload_pre_keys",
        "solana_call",
        "streams_start",
        "streams_stop",
        "users_confirm_email_verification",
        "users_start_email_verification",
        "users_update_profile",
    ];
    if WRITE_FUNCTIONS.contains(&function) {
        return true;
    }
    // Fail-closed against classification drift: a controller added after this
    // list whose name implies mutation is gated as a write rather than slipping
    // through un-prompted. Read verbs (get/list/resolve/…) don't match.
    // Verb fragments are matched as substrings, so they MUST NOT appear inside a
    // read controller name. Mutating actions that would collide with a read
    // (e.g. `marketplace_bid` vs `marketplace_list_bids`, `feeds_create_post` vs
    // `graphql_post`, `broadcasts_subscribe` vs `broadcasts_subscribers`) are
    // covered by the explicit WRITE_FUNCTIONS list above, so their bare verbs are
    // intentionally omitted here to avoid gating list reads.
    const WRITE_VERBS: &[&str] = &[
        "create",
        "update",
        "delete",
        "remove",
        "add_",
        "set_",
        "send",
        "join",
        "leave",
        "buy",
        "apply",
        "submit",
        "approve",
        "cancel",
        "register",
        "provision",
        "rotate",
        "mark_",
        "archive",
        "redeem",
        "revoke",
        "start",
        "stop",
        "confirm",
        "upload",
        "select",
        "shortlist",
        "withdraw",
        "vote",
        "adjudicate",
        "dispute",
        "council",
        "acknowledge",
        "fanout",
        "enforce",
        "renew",
        "transfer",
        "claim",
    ];
    WRITE_VERBS.iter().any(|verb| function.contains(verb))
}

/// The raw escape-hatch tool. Holds the controller handler table, keyed by the
/// bare function name (e.g. `directory_resolve`).
pub struct RawCallTool {
    handlers: HashMap<&'static str, ControllerHandler>,
}

impl RawCallTool {
    pub fn new() -> Self {
        let handlers = crate::openhuman::tinyplace::all_tinyplace_registered_controllers()
            .into_iter()
            .map(|c: RegisteredController| (c.schema.function, c.handler))
            .collect();
        Self { handlers }
    }

    pub fn boxed() -> Box<dyn Tool> {
        Box::new(Self::new())
    }

    fn command<'a>(&self, args: &'a Value) -> Option<&'a str> {
        args.get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

#[async_trait]
impl Tool for RawCallTool {
    fn name(&self) -> &str {
        "tinyplace_call"
    }

    fn description(&self) -> &str {
        "Escape hatch: invoke any tiny.place controller by name for the long \
         tail the curated flows don't cover. `command` is the bare function name \
         (e.g. 'directory_resolve', 'bounties_get', 'inbox_list'); `params` is its \
         JSON argument object. Run `tinyplace_help` with topic='commands' for the \
         full catalog. Read commands run immediately; write commands are gated."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Bare controller function name, e.g. 'directory_resolve'."
                },
                "params": {
                    "type": "object",
                    "description": "JSON argument object for the command (keys are camelCase).",
                    "additionalProperties": true
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let command = match self.command(&args) {
            Some(c) => c.to_string(),
            None => return Ok(err_md("## Missing command\n\nPass `command` — the bare controller function name. See `tinyplace_help` topic='commands'.".to_string())),
        };

        let params: Map<String, Value> = match args.get("params") {
            Some(Value::Object(map)) => map.clone(),
            None | Some(Value::Null) => Map::new(),
            Some(other) => {
                return Ok(err_md(format!(
                    "## Bad params\n\n`params` must be a JSON object, got {}.",
                    kind(other)
                )))
            }
        };

        let Some(handler) = self.handlers.get(command.as_str()) else {
            let mut md = Markdown::new();
            md.heading("Unknown command");
            md.paragraph(format!(
                "`{command}` is not a tiny.place controller. Run `tinyplace_help` \
                 with topic='commands' for the catalog."
            ));
            return Ok(err_md(md.build()));
        };

        log::debug!(
            "{LOG_PREFIX} command={command} write={} param_keys={:?}",
            is_write_function(&command),
            params.keys().collect::<Vec<_>>()
        );

        match handler(params).await {
            Ok(value) => {
                let mut md = Markdown::new();
                md.heading(format!("tiny.place · {command}"));
                md.raw_section(render_json(&value));
                Ok(ok_md(md.build()))
            }
            Err(message) => {
                // Controller errors are already strings; payment challenges come
                // through as the `PAYMENT_REQUIRED:` prefix the renderer parses.
                let mut md = Markdown::new();
                md.heading("Command failed");
                md.kv([("Command", command.clone()), ("Reason", message)]);
                Ok(err_md(md.build()))
            }
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        // Minimum across all commands: reads need only ReadOnly. The per-call
        // level is refined in `permission_level_with_args`.
        PermissionLevel::ReadOnly
    }

    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        match self.command(args) {
            Some(cmd) if is_write_function(cmd) => PermissionLevel::Write,
            _ => PermissionLevel::ReadOnly,
        }
    }

    fn external_effect(&self) -> bool {
        false
    }

    fn external_effect_with_args(&self, args: &Value) -> bool {
        self.command(args).map(is_write_function).unwrap_or(false)
    }

    fn is_concurrency_safe(&self, args: &Value) -> bool {
        !self.command(args).map(is_write_function).unwrap_or(true)
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    fn max_result_size_chars(&self) -> Option<usize> {
        Some(48 * 1024)
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_commands_gate_reads_do_not() {
        let tool = RawCallTool::new();
        let write = json!({ "command": "bounties_create" });
        let read = json!({ "command": "bounties_get" });
        assert_eq!(
            tool.permission_level_with_args(&write),
            PermissionLevel::Write
        );
        assert!(tool.external_effect_with_args(&write));
        assert_eq!(
            tool.permission_level_with_args(&read),
            PermissionLevel::ReadOnly
        );
        assert!(!tool.external_effect_with_args(&read));
        assert!(tool.is_concurrency_safe(&read));
    }

    #[test]
    fn unknown_mutating_commands_fail_closed_to_write() {
        // A controller not in the explicit list but whose name implies mutation
        // is gated as a write (fail-closed), while an unknown read stays read.
        assert!(is_write_function("widgets_create"));
        assert!(is_write_function("foo_delete"));
        assert!(is_write_function("thing_send_message"));
        assert!(!is_write_function("widgets_get"));
        assert!(!is_write_function("widgets_list"));
        assert!(!is_write_function("directory_resolve"));
    }

    #[test]
    fn list_reads_are_not_gated_by_verb_substrings() {
        // These are read controllers whose names embed mutating-verb fragments;
        // they must stay reads (the real write variants are in WRITE_FUNCTIONS).
        for read in [
            "marketplace_list_bids",
            "marketplace_list_offers",
            "graphql_identity_bids",
            "graphql_identity_offers",
            "graphql_posts",
            "graphql_post_comments",
            "broadcasts_subscribers",
        ] {
            assert!(!is_write_function(read), "{read} should be a read");
        }
        // …while the actual mutations remain writes (explicit list).
        for write in ["marketplace_bid", "marketplace_offer", "feeds_create_post"] {
            assert!(is_write_function(write), "{write} should be a write");
        }
    }

    #[test]
    fn handler_table_covers_known_controllers() {
        let tool = RawCallTool::new();
        for cmd in [
            "directory_resolve",
            "bounties_get",
            "inbox_list",
            "registry_register",
        ] {
            assert!(tool.handlers.contains_key(cmd), "missing controller {cmd}");
        }
    }

    #[tokio::test]
    async fn unknown_command_is_a_clean_error() {
        let tool = RawCallTool::new();
        let result = tool
            .execute(json!({ "command": "does_not_exist" }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.text().contains("Unknown command"));
    }
}
