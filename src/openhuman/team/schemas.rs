use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamIdParams {
    team_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTeamParams {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTeamParams {
    team_id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinTeamParams {
    code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveMemberParams {
    team_id: String,
    user_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InviteParams {
    team_id: String,
    #[serde(default)]
    max_uses: Option<u64>,
    #[serde(default)]
    expires_in_days: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangeRoleParams {
    team_id: String,
    user_id: String,
    role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RevokeInviteParams {
    team_id: String,
    invite_id: String,
}

pub fn all_team_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        team_schemas("team_get_usage"),
        team_schemas("team_list_members"),
        team_schemas("team_list_teams"),
        team_schemas("team_get_team"),
        team_schemas("team_create_team"),
        team_schemas("team_update_team"),
        team_schemas("team_delete_team"),
        team_schemas("team_switch_team"),
        team_schemas("team_leave_team"),
        team_schemas("team_join_team"),
        team_schemas("team_create_invite"),
        team_schemas("team_list_invites"),
        team_schemas("team_revoke_invite"),
        team_schemas("team_remove_member"),
        team_schemas("team_change_member_role"),
    ]
}

pub fn all_team_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: team_schemas("team_get_usage"),
            handler: handle_team_get_usage,
        },
        RegisteredController {
            schema: team_schemas("team_list_members"),
            handler: handle_team_list_members,
        },
        RegisteredController {
            schema: team_schemas("team_list_teams"),
            handler: handle_team_list_teams,
        },
        RegisteredController {
            schema: team_schemas("team_get_team"),
            handler: handle_team_get_team,
        },
        RegisteredController {
            schema: team_schemas("team_create_team"),
            handler: handle_team_create_team,
        },
        RegisteredController {
            schema: team_schemas("team_update_team"),
            handler: handle_team_update_team,
        },
        RegisteredController {
            schema: team_schemas("team_delete_team"),
            handler: handle_team_delete_team,
        },
        RegisteredController {
            schema: team_schemas("team_switch_team"),
            handler: handle_team_switch_team,
        },
        RegisteredController {
            schema: team_schemas("team_leave_team"),
            handler: handle_team_leave_team,
        },
        RegisteredController {
            schema: team_schemas("team_join_team"),
            handler: handle_team_join_team,
        },
        RegisteredController {
            schema: team_schemas("team_create_invite"),
            handler: handle_team_create_invite,
        },
        RegisteredController {
            schema: team_schemas("team_list_invites"),
            handler: handle_team_list_invites,
        },
        RegisteredController {
            schema: team_schemas("team_revoke_invite"),
            handler: handle_team_revoke_invite,
        },
        RegisteredController {
            schema: team_schemas("team_remove_member"),
            handler: handle_team_remove_member,
        },
        RegisteredController {
            schema: team_schemas("team_change_member_role"),
            handler: handle_team_change_member_role,
        },
    ]
}

pub fn team_schemas(function: &str) -> ControllerSchema {
    match function {
        "team_get_usage" => ControllerSchema {
            namespace: "team",
            function: "get_usage",
            description: "Fetch the current authenticated user's active team usage.",
            inputs: vec![],
            outputs: vec![json_output(
                "result",
                "Raw usage payload returned by /teams/me/usage.",
            )],
        },
        "team_list_members" => ControllerSchema {
            namespace: "team",
            function: "list_members",
            description: "List members for a team.",
            inputs: vec![required_string("teamId", "Team id.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Raw member array returned by /teams/:teamId/members.",
                required: true,
            }],
        },
        "team_list_teams" => ControllerSchema {
            namespace: "team",
            function: "list_teams",
            description: "List teams for the authenticated user.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Raw team array returned by GET /teams.",
                required: true,
            }],
        },
        "team_get_team" => ControllerSchema {
            namespace: "team",
            function: "get_team",
            description: "Fetch a single team.",
            inputs: vec![required_string("teamId", "Team id.")],
            outputs: vec![json_output(
                "result",
                "Raw team object returned by GET /teams/:teamId.",
            )],
        },
        "team_create_team" => ControllerSchema {
            namespace: "team",
            function: "create_team",
            description: "Create a team.",
            inputs: vec![required_string("name", "Team name.")],
            outputs: vec![json_output(
                "result",
                "Raw team object returned by POST /teams.",
            )],
        },
        "team_update_team" => ControllerSchema {
            namespace: "team",
            function: "update_team",
            description: "Update team fields.",
            inputs: vec![
                required_string("teamId", "Team id."),
                optional_string("name", "Updated team name."),
            ],
            outputs: vec![json_output(
                "result",
                "Raw team object returned by PUT /teams/:teamId.",
            )],
        },
        "team_delete_team" => ControllerSchema {
            namespace: "team",
            function: "delete_team",
            description: "Delete a team.",
            inputs: vec![required_string("teamId", "Team id.")],
            outputs: vec![json_output(
                "result",
                "Delete result returned by DELETE /teams/:teamId.",
            )],
        },
        "team_switch_team" => ControllerSchema {
            namespace: "team",
            function: "switch_team",
            description: "Switch the active team for the current user.",
            inputs: vec![required_string("teamId", "Team id.")],
            outputs: vec![json_output(
                "result",
                "Switch result returned by POST /teams/:teamId/switch.",
            )],
        },
        "team_leave_team" => ControllerSchema {
            namespace: "team",
            function: "leave_team",
            description: "Leave a team.",
            inputs: vec![required_string("teamId", "Team id.")],
            outputs: vec![json_output(
                "result",
                "Leave result returned by POST /teams/:teamId/leave.",
            )],
        },
        "team_join_team" => ControllerSchema {
            namespace: "team",
            function: "join_team",
            description: "Join a team using an invite code.",
            inputs: vec![required_string("code", "Invite code.")],
            outputs: vec![json_output(
                "result",
                "Raw team object returned by POST /teams/join.",
            )],
        },
        "team_create_invite" => ControllerSchema {
            namespace: "team",
            function: "create_invite",
            description: "Create an invite for a team.",
            inputs: vec![
                required_string("teamId", "Team id."),
                optional_u64("maxUses", "Optional max uses."),
                optional_u64("expiresInDays", "Optional expiry in days."),
            ],
            outputs: vec![json_output(
                "result",
                "Raw invite object returned by /teams/:teamId/invites.",
            )],
        },
        "team_remove_member" => ControllerSchema {
            namespace: "team",
            function: "remove_member",
            description: "Remove a member from a team.",
            inputs: vec![
                required_string("teamId", "Team id."),
                required_string("userId", "User id to remove."),
            ],
            outputs: vec![json_output(
                "result",
                "Removal result payload from /teams/:teamId/members/:userId.",
            )],
        },
        "team_change_member_role" => ControllerSchema {
            namespace: "team",
            function: "change_member_role",
            description: "Change a member's role in a team.",
            inputs: vec![
                required_string("teamId", "Team id."),
                required_string("userId", "User id."),
                required_string("role", "Role identifier."),
            ],
            outputs: vec![json_output(
                "result",
                "Role update payload from /teams/:teamId/members/:userId/role.",
            )],
        },
        "team_list_invites" => ControllerSchema {
            namespace: "team",
            function: "list_invites",
            description: "List active invites for a team.",
            inputs: vec![required_string("teamId", "Team id.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Raw invite array returned by /teams/:teamId/invites.",
                required: true,
            }],
        },
        "team_revoke_invite" => ControllerSchema {
            namespace: "team",
            function: "revoke_invite",
            description: "Revoke (delete) an existing team invite.",
            inputs: vec![
                required_string("teamId", "Team id."),
                required_string("inviteId", "Invite id to revoke."),
            ],
            outputs: vec![json_output(
                "result",
                "Revoke result from /teams/:teamId/invites/:inviteId.",
            )],
        },
        _ => ControllerSchema {
            namespace: "team",
            function: "unknown",
            description: "Unknown team controller.",
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

fn handle_team_get_usage(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::team::get_usage(&config).await?)
    })
}

fn handle_team_list_members(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<TeamIdParams>(params)?;
        to_json(crate::openhuman::team::list_members(&config, &payload.team_id).await?)
    })
}

fn handle_team_list_teams(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::team::list_teams(&config).await?)
    })
}

fn handle_team_get_team(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<TeamIdParams>(params)?;
        to_json(crate::openhuman::team::get_team(&config, &payload.team_id).await?)
    })
}

fn handle_team_create_team(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<CreateTeamParams>(params)?;
        to_json(crate::openhuman::team::create_team(&config, &payload.name).await?)
    })
}

fn handle_team_update_team(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<UpdateTeamParams>(params)?;
        to_json(
            crate::openhuman::team::update_team(&config, &payload.team_id, payload.name.as_deref())
                .await?,
        )
    })
}

fn handle_team_delete_team(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<TeamIdParams>(params)?;
        to_json(crate::openhuman::team::delete_team(&config, &payload.team_id).await?)
    })
}

fn handle_team_switch_team(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<TeamIdParams>(params)?;
        to_json(crate::openhuman::team::switch_team(&config, &payload.team_id).await?)
    })
}

fn handle_team_leave_team(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<TeamIdParams>(params)?;
        to_json(crate::openhuman::team::leave_team(&config, &payload.team_id).await?)
    })
}

fn handle_team_join_team(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<JoinTeamParams>(params)?;
        to_json(crate::openhuman::team::join_team(&config, &payload.code).await?)
    })
}

fn handle_team_create_invite(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<InviteParams>(params)?;
        to_json(
            crate::openhuman::team::create_invite(
                &config,
                &payload.team_id,
                payload.max_uses,
                payload.expires_in_days,
            )
            .await?,
        )
    })
}

fn handle_team_remove_member(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RemoveMemberParams>(params)?;
        to_json(
            crate::openhuman::team::remove_member(&config, &payload.team_id, &payload.user_id)
                .await?,
        )
    })
}

fn handle_team_change_member_role(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<ChangeRoleParams>(params)?;
        to_json(
            crate::openhuman::team::change_member_role(
                &config,
                &payload.team_id,
                &payload.user_id,
                &payload.role,
            )
            .await?,
        )
    })
}

fn handle_team_list_invites(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<TeamIdParams>(params)?;
        to_json(crate::openhuman::team::list_invites(&config, &payload.team_id).await?)
    })
}

fn handle_team_revoke_invite(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload = deserialize_params::<RevokeInviteParams>(params)?;
        to_json(
            crate::openhuman::team::revoke_invite(&config, &payload.team_id, &payload.invite_id)
                .await?,
        )
    })
}

fn to_json(outcome: RpcOutcome<Value>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_names_are_stable() {
        let s = team_schemas("team_list_members");
        assert_eq!(s.namespace, "team");
        assert_eq!(s.function, "list_members");
    }

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_team_controller_schemas().len(),
            all_team_registered_controllers().len()
        );
    }

    #[test]
    fn schemas_match_unwrapped_team_payload_shapes() {
        let members = team_schemas("team_list_members");
        assert_eq!(members.outputs.len(), 1);
        assert_eq!(members.outputs[0].name, "result");
        assert_eq!(
            members.outputs[0].ty,
            TypeSchema::Array(Box::new(TypeSchema::Json))
        );

        let create_invite = team_schemas("team_create_invite");
        assert_eq!(create_invite.outputs.len(), 1);
        assert_eq!(create_invite.outputs[0].name, "result");
        assert_eq!(create_invite.outputs[0].ty, TypeSchema::Json);

        let invites = team_schemas("team_list_invites");
        assert_eq!(invites.outputs.len(), 1);
        assert_eq!(invites.outputs[0].name, "result");
        assert_eq!(
            invites.outputs[0].ty,
            TypeSchema::Array(Box::new(TypeSchema::Json))
        );
    }
}
