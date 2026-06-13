//! LLM-callable wrappers over the `team` domain.
//!
//! Reads (list teams/members/invites, get team, usage) are default-ON. Every
//! membership/org mutator ships default-OFF via `tools/user_filter.rs`
//! (`team_admin` toggle); delete_team and remove_member are `Dangerous`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::team;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

fn req_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

fn team_id_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": { "team_id": { "type": "string" } },
        "required": ["team_id"]
    })
}

/// `&Config`-only read.
macro_rules! cfg_read {
    ($ty:ident, $name:literal, $fn:ident, $desc:literal) => {
        pub struct $ty {
            config: Arc<Config>,
        }
        impl $ty {
            pub fn new(config: Arc<Config>) -> Self {
                Self { config }
            }
        }
        #[async_trait]
        impl Tool for $ty {
            fn name(&self) -> &str {
                $name
            }
            fn description(&self) -> &str {
                $desc
            }
            fn parameters_schema(&self) -> serde_json::Value {
                json!({ "type": "object", "properties": {} })
            }
            async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
                emit!(team::$fn(&self.config).await, $name)
            }
            fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
                true
            }
        }
    };
}

/// `(config, team_id)` read.
macro_rules! team_id_read {
    ($ty:ident, $name:literal, $fn:ident, $desc:literal) => {
        pub struct $ty {
            config: Arc<Config>,
        }
        impl $ty {
            pub fn new(config: Arc<Config>) -> Self {
                Self { config }
            }
        }
        #[async_trait]
        impl Tool for $ty {
            fn name(&self) -> &str {
                $name
            }
            fn description(&self) -> &str {
                $desc
            }
            fn parameters_schema(&self) -> serde_json::Value {
                team_id_schema()
            }
            async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
                let team_id = req_str(&args, "team_id")?;
                emit!(team::$fn(&self.config, &team_id).await, $name)
            }
            fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
                true
            }
        }
    };
}

cfg_read!(
    TeamListTool,
    "team_list",
    list_teams,
    "List the teams the user belongs to."
);
cfg_read!(
    TeamUsageTool,
    "team_get_usage",
    get_usage,
    "Return usage metrics for the active team."
);
team_id_read!(
    TeamGetTool,
    "team_get",
    get_team,
    "Get one team by `team_id`."
);
team_id_read!(
    TeamListMembersTool,
    "team_list_members",
    list_members,
    "List the members of a team by `team_id`."
);
team_id_read!(
    TeamListInvitesTool,
    "team_list_invites",
    list_invites,
    "List the outstanding invites for a team by `team_id`."
);

/// `(config, team_id)` mutator at a given permission level.
macro_rules! team_id_mutator {
    ($ty:ident, $name:literal, $fn:ident, $perm:expr, $desc:literal) => {
        pub struct $ty {
            config: Arc<Config>,
        }
        impl $ty {
            pub fn new(config: Arc<Config>) -> Self {
                Self { config }
            }
        }
        #[async_trait]
        impl Tool for $ty {
            fn name(&self) -> &str {
                $name
            }
            fn description(&self) -> &str {
                $desc
            }
            fn parameters_schema(&self) -> serde_json::Value {
                team_id_schema()
            }
            fn permission_level(&self) -> PermissionLevel {
                $perm
            }
            async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
                let team_id = req_str(&args, "team_id")?;
                emit!(team::$fn(&self.config, &team_id).await, $name)
            }
        }
    };
}

team_id_mutator!(
    TeamDeleteTool,
    "team_delete",
    delete_team,
    PermissionLevel::Dangerous,
    "Delete a team by `team_id`. Default-OFF (opt-in)."
);
team_id_mutator!(
    TeamSwitchTool,
    "team_switch",
    switch_team,
    PermissionLevel::Write,
    "Switch the active team to `team_id`. Default-OFF (opt-in)."
);
team_id_mutator!(
    TeamLeaveTool,
    "team_leave",
    leave_team,
    PermissionLevel::Write,
    "Leave a team by `team_id`. Default-OFF (opt-in)."
);

/// Create a team.
pub struct TeamCreateTool {
    config: Arc<Config>,
}
impl TeamCreateTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for TeamCreateTool {
    fn name(&self) -> &str {
        "team_create"
    }
    fn description(&self) -> &str {
        "Create a new team with `name`. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let name = req_str(&args, "name")?;
        emit!(team::create_team(&self.config, &name).await, "team_create")
    }
}

/// Update a team's name.
pub struct TeamUpdateTool {
    config: Arc<Config>,
}
impl TeamUpdateTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for TeamUpdateTool {
    fn name(&self) -> &str {
        "team_update"
    }
    fn description(&self) -> &str {
        "Update a team (`team_id`) name. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "team_id": { "type": "string" }, "name": { "type": "string" } },
            "required": ["team_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let team_id = req_str(&args, "team_id")?;
        let name = args.get("name").and_then(Value::as_str);
        emit!(
            team::update_team(&self.config, &team_id, name).await,
            "team_update"
        )
    }
}

/// Join a team via invite code.
pub struct TeamJoinTool {
    config: Arc<Config>,
}
impl TeamJoinTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for TeamJoinTool {
    fn name(&self) -> &str {
        "team_join"
    }
    fn description(&self) -> &str {
        "Join a team via an invite `code`. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "code": { "type": "string" } }, "required": ["code"] })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let code = req_str(&args, "code")?;
        emit!(team::join_team(&self.config, &code).await, "team_join")
    }
}

/// Create a team invite.
pub struct TeamCreateInviteTool {
    config: Arc<Config>,
}
impl TeamCreateInviteTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for TeamCreateInviteTool {
    fn name(&self) -> &str {
        "team_create_invite"
    }
    fn description(&self) -> &str {
        "Create an invite for a team (`team_id`), optional `max_uses` and \
         `expires_in_days`. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "team_id": { "type": "string" },
                "max_uses": { "type": "integer", "minimum": 1 },
                "expires_in_days": { "type": "integer", "minimum": 1 }
            },
            "required": ["team_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let team_id = req_str(&args, "team_id")?;
        let max_uses = args.get("max_uses").and_then(Value::as_u64);
        let expires = args.get("expires_in_days").and_then(Value::as_u64);
        emit!(
            team::create_invite(&self.config, &team_id, max_uses, expires).await,
            "team_create_invite"
        )
    }
}

/// Revoke a team invite.
pub struct TeamRevokeInviteTool {
    config: Arc<Config>,
}
impl TeamRevokeInviteTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for TeamRevokeInviteTool {
    fn name(&self) -> &str {
        "team_revoke_invite"
    }
    fn description(&self) -> &str {
        "Revoke a team invite (`team_id` + `invite_id`). Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "team_id": { "type": "string" }, "invite_id": { "type": "string" } },
            "required": ["team_id", "invite_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let team_id = req_str(&args, "team_id")?;
        let invite_id = req_str(&args, "invite_id")?;
        emit!(
            team::revoke_invite(&self.config, &team_id, &invite_id).await,
            "team_revoke_invite"
        )
    }
}

/// Remove a team member.
pub struct TeamRemoveMemberTool {
    config: Arc<Config>,
}
impl TeamRemoveMemberTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for TeamRemoveMemberTool {
    fn name(&self) -> &str {
        "team_remove_member"
    }
    fn description(&self) -> &str {
        "Remove a member (`user_id`) from a team (`team_id`). Default-OFF \
         (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "team_id": { "type": "string" }, "user_id": { "type": "string" } },
            "required": ["team_id", "user_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let team_id = req_str(&args, "team_id")?;
        let user_id = req_str(&args, "user_id")?;
        emit!(
            team::remove_member(&self.config, &team_id, &user_id).await,
            "team_remove_member"
        )
    }
}

/// Change a team member's role.
pub struct TeamChangeMemberRoleTool {
    config: Arc<Config>,
}
impl TeamChangeMemberRoleTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for TeamChangeMemberRoleTool {
    fn name(&self) -> &str {
        "team_change_member_role"
    }
    fn description(&self) -> &str {
        "Change a team member's `role` (`team_id` + `user_id` + `role`). \
         Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "team_id": { "type": "string" },
                "user_id": { "type": "string" },
                "role": { "type": "string" }
            },
            "required": ["team_id", "user_id", "role"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let team_id = req_str(&args, "team_id")?;
        let user_id = req_str(&args, "user_id")?;
        let role = req_str(&args, "role")?;
        emit!(
            team::change_member_role(&self.config, &team_id, &user_id, &role).await,
            "team_change_member_role"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    fn cfg() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn names_and_levels() {
        assert_eq!(TeamListTool::new(cfg()).name(), "team_list");
        assert_eq!(
            TeamListTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            TeamCreateTool::new(cfg()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(
            TeamDeleteTool::new(cfg()).permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(
            TeamRemoveMemberTool::new(cfg()).permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(TeamListTool::new(cfg()).scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn get_requires_team_id() {
        let err = TeamGetTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing team_id");
        assert!(err.to_string().contains("team_id"));
    }
}
