//! Controller schemas + JSON-RPC dispatchers for durable agent-team
//! coordination (#3374). Namespace `agent_team`.
//!
//! Surface: create a team with members, list/get teams, assign dependency-aware
//! tasks, atomically claim a task, exchange + list teammate messages, complete a
//! claimed task behind a quality gate, shut a member down (releasing its tasks),
//! close a team, and — via `start_member` — spawn a live worker that claims and
//! runs a task to completion (see [`super::runtime`]).

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::session_db::run_ledger::AgentTeamListRequest;
use crate::rpc::RpcOutcome;

use super::ops::{self, NewMember};
use super::runtime;

/// Controller schemas exposed by the agent-teams module.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("agent_team_create"),
        schema_for("agent_team_list"),
        schema_for("agent_team_get"),
        schema_for("agent_team_assign_task"),
        schema_for("agent_team_claim_task"),
        schema_for("agent_team_message_member"),
        schema_for("agent_team_list_messages"),
        schema_for("agent_team_complete_task"),
        schema_for("agent_team_shutdown_member"),
        schema_for("agent_team_close"),
        schema_for("agent_team_start_member"),
    ]
}

/// Registered controllers (schema + handler) for agent teams.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("agent_team_create"),
            handler: handle_create,
        },
        RegisteredController {
            schema: schema_for("agent_team_list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schema_for("agent_team_get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schema_for("agent_team_assign_task"),
            handler: handle_assign_task,
        },
        RegisteredController {
            schema: schema_for("agent_team_claim_task"),
            handler: handle_claim_task,
        },
        RegisteredController {
            schema: schema_for("agent_team_message_member"),
            handler: handle_message_member,
        },
        RegisteredController {
            schema: schema_for("agent_team_list_messages"),
            handler: handle_list_messages,
        },
        RegisteredController {
            schema: schema_for("agent_team_complete_task"),
            handler: handle_complete_task,
        },
        RegisteredController {
            schema: schema_for("agent_team_shutdown_member"),
            handler: handle_shutdown_member,
        },
        RegisteredController {
            schema: schema_for("agent_team_close"),
            handler: handle_close,
        },
        RegisteredController {
            schema: schema_for("agent_team_start_member"),
            handler: handle_start_member,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "agent_team_create" => ControllerSchema {
            namespace: "agent_team",
            function: "create",
            description: "Create an agent team with a lead and initial members.",
            inputs: vec![
                required_str("leadAgentId", "Lead agent id coordinating the team."),
                optional_str("parentThreadId", "Originating chat thread id."),
                optional_str("summary", "Short description of the team's goal."),
                json_input(
                    "members",
                    "Array of { name, agentId? } members to seed the team.",
                ),
            ],
            outputs: vec![json_output("result", "TeamView with team, members, tasks.")],
        },
        "agent_team_list" => ControllerSchema {
            namespace: "agent_team",
            function: "list",
            description: "List durable agent teams with optional filters and pagination.",
            inputs: vec![
                optional_str("parentThreadId", "Filter by parent thread id."),
                optional_str("status", "Filter by team status (active|closed)."),
                optional_u64("limit", "Max teams to return (default 50, max 500)."),
                optional_u64("offset", "Pagination offset."),
            ],
            outputs: vec![json_output(
                "result",
                "AgentTeamListResponse with teams array and count.",
            )],
        },
        "agent_team_get" => ControllerSchema {
            namespace: "agent_team",
            function: "get",
            description: "Get a team with its members and tasks.",
            inputs: vec![required_str("teamId", "Team id.")],
            outputs: vec![json_output("team", "TeamView payload or null.")],
        },
        "agent_team_assign_task" => ControllerSchema {
            namespace: "agent_team",
            function: "assign_task",
            description: "Add a dependency-aware task to a team.",
            inputs: vec![
                required_str("teamId", "Team id."),
                required_str("title", "Task title."),
                optional_str("objective", "Task objective / acceptance summary."),
                optional_str("ownerMemberId", "Member who owns the task."),
                json_input("dependsOn", "Array of task ids this task depends on."),
            ],
            outputs: vec![json_output("task", "The created AgentTeamTask.")],
        },
        "agent_team_claim_task" => ControllerSchema {
            namespace: "agent_team",
            function: "claim_task",
            description: "Atomically claim a task for a member (race-safe compare-and-swap).",
            inputs: vec![
                required_str("teamId", "Team id."),
                required_str("taskId", "Task id to claim."),
                required_str("memberId", "Member attempting the claim."),
                required_str("claimToken", "Idempotency / ownership token for the claim."),
            ],
            outputs: vec![json_output(
                "result",
                "ClaimOutcome: claimed | alreadyClaimed | blocked | unknownTask.",
            )],
        },
        "agent_team_message_member" => ControllerSchema {
            namespace: "agent_team",
            function: "message_member",
            description: "Send a message from one member to another (or broadcast to the team).",
            inputs: vec![
                required_str("teamId", "Team id."),
                optional_str(
                    "fromMemberId",
                    "Sender member id; omit for a lead/user-originated message.",
                ),
                optional_str("toMemberId", "Recipient member id (omit to broadcast)."),
                required_str("content", "Message content."),
                optional_str("visibility", "Message visibility (default team)."),
            ],
            outputs: vec![json_output("message", "The appended message event.")],
        },
        "agent_team_list_messages" => ControllerSchema {
            namespace: "agent_team",
            function: "list_messages",
            description: "List the team's messages in order.",
            inputs: vec![
                required_str("teamId", "Team id."),
                optional_u64("limit", "Max messages to return."),
            ],
            outputs: vec![json_output("messages", "Array of message events.")],
        },
        "agent_team_complete_task" => ControllerSchema {
            namespace: "agent_team",
            function: "complete_task",
            description:
                "Complete a claimed task, gating its transition to done behind quality checks.",
            inputs: vec![
                required_str("teamId", "Team id."),
                required_str("taskId", "Task id to complete."),
                required_str(
                    "memberId",
                    "Member completing the task (must be the claimant).",
                ),
                json_input(
                    "evidence",
                    "Array of evidence links to attach on completion.",
                ),
                optional_bool(
                    "requireEvidence",
                    "Require at least one evidence link to pass the gate (default false).",
                ),
            ],
            outputs: vec![json_output(
                "result",
                "CompletionOutcome: completed | gateFailed | notClaimed | unknownTask.",
            )],
        },
        "agent_team_shutdown_member" => ControllerSchema {
            namespace: "agent_team",
            function: "shutdown_member",
            description: "Stop a team member and release any task it is actively working on.",
            inputs: vec![
                required_str("teamId", "Team id."),
                required_str("memberId", "Member id to stop."),
            ],
            outputs: vec![json_output(
                "result",
                "MemberShutdown: the stopped member and releasedTaskIds.",
            )],
        },
        "agent_team_close" => ControllerSchema {
            namespace: "agent_team",
            function: "close",
            description: "Close a team, recording an optional summary.",
            inputs: vec![
                required_str("teamId", "Team id."),
                optional_str("summary", "Closing summary."),
            ],
            outputs: vec![json_output("team", "The closed AgentTeam.")],
        },
        "agent_team_start_member" => ControllerSchema {
            namespace: "agent_team",
            function: "start_member",
            description: "Spawn a live worker for a member: claims a task and runs a real sub-agent to completion.",
            inputs: vec![
                required_str("teamId", "Team id."),
                required_str("memberId", "Member to run."),
                optional_str(
                    "taskId",
                    "Specific task id; omit to auto-pick the member's next claimable task.",
                ),
                optional_str(
                    "modelOverride",
                    "Test-only: pin the spawned worker to this model (production omits).",
                ),
            ],
            outputs: vec![json_output(
                "result",
                "StartMemberOutcome: started | blocked | alreadyClaimed | noClaimableTask | unknownTask.",
            )],
        },
        other => unreachable!("unknown agent_team schema: {other}"),
    }
}

fn handle_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] create.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] create.config_failed err={err}");
        })?;
        let lead = require_str(&params, "leadAgentId")?;
        let parent_thread_id = opt_str(&params, "parentThreadId");
        let summary = opt_str(&params, "summary");
        let members = parse_members(&params)?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] create.parsed lead={lead} members={}", members.len());
        let view = ops::create_team(
            &config,
            &lead,
            parent_thread_id.as_deref(),
            summary.as_deref(),
            &members,
        )
        .map_err(|e| log_err(&cid, "create", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] create.success id={}", view.team.id);
        to_json(view)
    })
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] list.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] list.config_failed err={err}");
        })?;
        let request: AgentTeamListRequest = if params.is_empty() {
            log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] list.branch=default_request");
            AgentTeamListRequest::default()
        } else {
            log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] list.branch=parsed_params");
            serde_json::from_value(Value::Object(params)).map_err(|e| {
                let s = format!("invalid agent team list params: {e}");
                log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] list.bad_params err={s}");
                s
            })?
        };
        let response = ops::list_teams(&config, &request).map_err(|e| log_err(&cid, "list", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] list.success count={}", response.count);
        to_json(response)
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] get.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] get.config_failed err={err}");
        })?;
        let team_id = require_str(&params, "teamId")?;
        let view = ops::get_team(&config, &team_id).map_err(|e| log_err(&cid, "get", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] get.success id={team_id} found={}", view.is_some());
        to_json(serde_json::json!({ "team": view }))
    })
}

fn handle_assign_task(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] assign_task.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] assign_task.config_failed err={err}");
        })?;
        let team_id = require_str(&params, "teamId")?;
        let title = require_str(&params, "title")?;
        let objective = opt_str(&params, "objective");
        let owner = opt_str(&params, "ownerMemberId");
        let depends_on = opt_str_array(&params, "dependsOn");
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] assign_task.parsed team={team_id} deps={}", depends_on.len());
        let task = ops::assign_task(
            &config,
            &team_id,
            &title,
            objective.as_deref(),
            owner.as_deref(),
            &depends_on,
        )
        .map_err(|e| log_err(&cid, "assign_task", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] assign_task.success task={}", task.id);
        to_json(serde_json::json!({ "task": task }))
    })
}

fn handle_claim_task(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] claim_task.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] claim_task.config_failed err={err}");
        })?;
        let team_id = require_str(&params, "teamId")?;
        let task_id = require_str(&params, "taskId")?;
        let member_id = require_str(&params, "memberId")?;
        let claim_token = require_str(&params, "claimToken")?;
        let outcome = ops::claim_task(&config, &team_id, &task_id, &member_id, &claim_token)
            .map_err(|e| log_err(&cid, "claim_task", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] claim_task.success team={team_id} task={task_id}");
        to_json(serde_json::json!({ "result": outcome }))
    })
}

fn handle_message_member(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] message_member.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] message_member.config_failed err={err}");
        })?;
        let team_id = require_str(&params, "teamId")?;
        // `fromMemberId` is optional: omitted ⇒ lead/user-originated message
        // (stored as `from = "lead"`); present ⇒ teammate-to-teammate.
        let from = opt_str(&params, "fromMemberId");
        let to = opt_str(&params, "toMemberId");
        let content = require_str(&params, "content")?;
        let visibility = opt_str(&params, "visibility");
        let event = ops::message_member(
            &config,
            &team_id,
            from.as_deref(),
            to.as_deref(),
            &content,
            visibility.as_deref(),
        )
        .map_err(|e| log_err(&cid, "message_member", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] message_member.success team={team_id} sequence={}", event.sequence);
        to_json(serde_json::json!({ "message": event }))
    })
}

fn handle_list_messages(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] list_messages.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] list_messages.config_failed err={err}");
        })?;
        let team_id = require_str(&params, "teamId")?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as u32);
        let messages = ops::list_messages(&config, &team_id, limit)
            .map_err(|e| log_err(&cid, "list_messages", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] list_messages.success team={team_id} count={}", messages.len());
        to_json(serde_json::json!({ "messages": messages }))
    })
}

fn handle_complete_task(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] complete_task.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] complete_task.config_failed err={err}");
        })?;
        let team_id = require_str(&params, "teamId")?;
        let task_id = require_str(&params, "taskId")?;
        let member_id = require_str(&params, "memberId")?;
        let evidence = opt_str_array(&params, "evidence");
        let require_evidence = params
            .get("requireEvidence")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] complete_task.parsed team={team_id} task={task_id} evidence={} requireEvidence={require_evidence}", evidence.len());
        let outcome = ops::complete_task(
            &config,
            &team_id,
            &task_id,
            &member_id,
            &evidence,
            require_evidence,
        )
        .map_err(|e| log_err(&cid, "complete_task", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] complete_task.success team={team_id} task={task_id}");
        to_json(serde_json::json!({ "result": outcome }))
    })
}

fn handle_shutdown_member(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] shutdown_member.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] shutdown_member.config_failed err={err}");
        })?;
        let team_id = require_str(&params, "teamId")?;
        let member_id = require_str(&params, "memberId")?;
        let result = ops::shutdown_member(&config, &team_id, &member_id)
            .map_err(|e| log_err(&cid, "shutdown_member", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] shutdown_member.success team={team_id} member={member_id} released={}", result.released_task_ids.len());
        to_json(serde_json::json!({ "result": result }))
    })
}

fn handle_close(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] close.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] close.config_failed err={err}");
        })?;
        let team_id = require_str(&params, "teamId")?;
        let summary = opt_str(&params, "summary");
        let team = ops::close_team(&config, &team_id, summary.as_deref())
            .map_err(|e| log_err(&cid, "close", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] close.success team={team_id}");
        to_json(serde_json::json!({ "team": team }))
    })
}

fn handle_start_member(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] start_member.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] start_member.config_failed err={err}");
        })?;
        let team_id = require_str(&params, "teamId")?;
        let member_id = require_str(&params, "memberId")?;
        let task_id = opt_str(&params, "taskId");
        // Test-only seam: pin the spawned worker to a model (e.g. the e2e mock).
        let model_override = opt_str(&params, "modelOverride");
        let outcome = runtime::start_member_run(
            &config,
            &team_id,
            &member_id,
            task_id.as_deref(),
            model_override,
        )
        .await
        .map_err(|e| log_err(&cid, "start_member", e))?;
        log::debug!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] start_member.success team={team_id} member={member_id}");
        to_json(serde_json::json!({ "result": outcome }))
    })
}

fn parse_members(params: &Map<String, Value>) -> Result<Vec<NewMember>, String> {
    let raw = match params.get("members") {
        Some(Value::Array(items)) => items,
        Some(Value::Null) | None => return Ok(vec![]),
        Some(_) => return Err("members must be an array".to_string()),
    };
    let mut members = Vec::with_capacity(raw.len());
    for item in raw {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "each member requires a non-empty name".to_string())?;
        let agent_id = item
            .get("agentId")
            .and_then(Value::as_str)
            .map(str::to_string);
        members.push(NewMember {
            name: name.to_string(),
            agent_id,
        });
    }
    Ok(members)
}

fn require_str(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing required param: {key}"))
}

fn opt_str(params: &Map<String, Value>, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

fn opt_str_array(params: &Map<String, Value>, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn log_err(cid: &str, op: &str, err: anyhow::Error) -> String {
    let s = err.to_string();
    log::warn!(target: "agent_team_rpc", "[agent_team_rpc][{cid}] {op}.error err={s}");
    s
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    RpcOutcome::new(value, vec![]).into_cli_compatible_json()
}

fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn required_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
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

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

fn json_input(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
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
    fn registered_controllers_match_schemas() {
        let schemas = all_controller_schemas();
        let registered = all_registered_controllers();
        assert_eq!(schemas.len(), registered.len());
        assert_eq!(schemas.len(), 11);
        assert!(schemas.iter().all(|s| s.namespace == "agent_team"));
        assert_eq!(schema_for("agent_team_claim_task").function, "claim_task");
        assert_eq!(
            schema_for("agent_team_complete_task").function,
            "complete_task"
        );
        assert_eq!(
            schema_for("agent_team_shutdown_member").function,
            "shutdown_member"
        );
    }
}
