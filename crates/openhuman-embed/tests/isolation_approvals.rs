//! Approvals on one runtime belong to the agent that parked them: each agent
//! lists and decides only its own, two agents on the same thread id never
//! answer each other, and switching one agent's gate off leaves its siblings
//! parking.

mod common;

use common::{
    eventually, offline_config, route, runtime, scripted_provider, stub_backend,
    tool_call_completion,
};
use openhuman_core::core::events::DomainEvent;
use openhuman_core::security::approval::{parse_approval_reply, ApprovalGate};
use openhuman_core::security::AutonomyLevel;
use openhuman_embed::{
    Access, Agent, AgentDefinitionSpec, AgentSpec, AgentTurnOrigin, ApprovalDecision,
    ApprovalsError, Runtime, ToolScopeSpec, TrustedAccess, Workspace,
};

const THREAD: &str = "shared-thread";
const SESSION: &str = "shared-session";

fn shell_writes(file: &std::path::Path) -> serde_json::Value {
    tool_call_completion(
        "shell",
        &serde_json::json!({ "command": format!("touch {}", file.display()) }).to_string(),
    )
}

fn supervised(scratch: &std::path::Path) -> Access {
    Access::supervised()
        .origin(AgentTurnOrigin::WebChat {
            thread_id: THREAD.to_string(),
            client_id: "client-shared".to_string(),
            request_id: None,
        })
        .auto_approve(Vec::<String>::new())
        .auto_approve_all(false)
        .trust(scratch.display().to_string(), TrustedAccess::ReadWrite)
}

async fn agent(
    runtime: &Runtime,
    id: &str,
    access: Access,
    file: &std::path::Path,
) -> (Agent, wiremock::MockServer) {
    let provider = scripted_provider(vec![shell_writes(file)], &format!("{id}-done")).await;
    let agent = runtime
        .agent(
            AgentSpec::new(id)
                .provider(route(&provider, &format!("{id}-model")))
                .access(access)
                .definition(
                    AgentDefinitionSpec::new()
                        .tools(ToolScopeSpec::Named(vec!["shell".to_string()])),
                ),
        )
        .expect("agent instantiates");
    (agent, provider)
}

fn start(agent: &Agent) -> tokio::task::JoinHandle<()> {
    let agent = agent.clone();
    tokio::spawn(async move {
        agent
            .turn("write the marker")
            .session(SESSION)
            .send()
            .await
            .expect("turn returns");
    })
}

async fn parked(agent: &Agent) -> openhuman_embed::PendingApproval {
    eventually(&format!("{} to park", agent.id()), || {
        agent
            .approvals()
            .pending()
            .ok()
            .and_then(|rows| rows.into_iter().next())
    })
    .await
}

#[test]
fn approvals_belong_to_the_agent_that_parked_them() {
    let _ = env_logger::builder().is_test(true).try_init();
    runtime().block_on(async {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let mut config = offline_config();
            config.autonomy.enabled = true;
            config.autonomy.level = AutonomyLevel::Full;
            config.autonomy.auto_approve_all = true;
            let runtime = Runtime::builder()
                .config(config)
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .access(Access::full())
                .build()
                .await
                .expect("runtime builds");
            let mut events = openhuman_core::core::bus::BUS
                .get()
                .expect("the runtime initialised the bus")
                .receiver();
            let scratch = tempfile::tempdir().expect("scratch dir");
            let p_file = scratch.path().join("p-wrote");
            let q_file = scratch.path().join("q-wrote");
            let (p, _p_provider) =
                agent(&runtime, "agent-p", supervised(scratch.path()), &p_file).await;
            let (q, _q_provider) =
                agent(&runtime, "agent-q", supervised(scratch.path()), &q_file).await;

            // ── two agents, one session and thread id, both park ──
            let p_turn = start(&p);
            let q_turn = start(&q);
            let p_request = parked(&p).await;
            let q_request = parked(&q).await;
            assert_ne!(p_request.request_id, q_request.request_id);
            assert_eq!(p.approvals().pending().unwrap().len(), 1);
            assert_eq!(q.approvals().pending().unwrap().len(), 1);
            assert_eq!(p_request.agent_id.as_deref(), Some("agent-p"));
            assert_eq!(q_request.agent_id.as_deref(), Some("agent-q"));

            // ── ApprovalRequested names the agent ──
            let mut requested = std::collections::HashMap::new();
            while requested.len() < 2 {
                match tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
                    .await
                    .expect("ApprovalRequested events arrive")
                {
                    Some(DomainEvent::ApprovalRequested {
                        request_id,
                        agent_id,
                        ..
                    }) => {
                        requested.insert(request_id, agent_id);
                    }
                    Some(_) => {}
                    None => panic!("bus closed"),
                }
            }
            assert_eq!(
                requested
                    .get(&p_request.request_id)
                    .cloned()
                    .flatten()
                    .as_deref(),
                Some("agent-p")
            );
            assert_eq!(
                requested
                    .get(&q_request.request_id)
                    .cloned()
                    .flatten()
                    .as_deref(),
                Some("agent-q")
            );

            // ── P cannot decide Q's request ──
            let refused = p
                .approvals()
                .decide(&q_request.request_id, ApprovalDecision::ApproveOnce)
                .expect_err("p must not decide q's request");
            assert!(
                matches!(refused, ApprovalsError::WrongAgent(_)),
                "{refused:?}"
            );
            assert_eq!(q.approvals().pending().unwrap().len(), 1, "q stays parked");

            // ── P's own decision releases only P ──
            p.approvals()
                .decide(&p_request.request_id, ApprovalDecision::ApproveOnce)
                .expect("p decides its own request");
            p_turn.await.unwrap();
            assert!(p_file.exists(), "p's approved call ran");
            assert!(!q_file.exists(), "q's call is still parked");
            assert!(!q_turn.is_finished());

            // ── a chat-style reply routes by agent and thread ──
            let gate = ApprovalGate::try_global().expect("the runtime installed the gate");
            assert!(
                gate.pending_for_thread(THREAD).is_none(),
                "a reply outside any agent reaches no agent's request"
            );
            assert!(gate
                .pending_for_agent_thread(Some("agent-p"), THREAD)
                .is_none());
            let routed = gate
                .pending_for_agent_thread(Some("agent-q"), THREAD)
                .expect("q's thread routes to q's request");
            assert_eq!(routed, q_request.request_id);
            let decision = parse_approval_reply("yes").expect("yes is an answer");
            q.approvals()
                .decide(&routed, decision)
                .expect("q decides through the routed reply");
            q_turn.await.unwrap();
            assert!(q_file.exists(), "q's approved call ran");

            // ── the gate switch is per agent ──
            let r_file = scratch.path().join("r-wrote");
            let s_file = scratch.path().join("s-wrote");
            let (r, _r_provider) = agent(
                &runtime,
                "gate-off-r",
                supervised(scratch.path()).approval_gate(false),
                &r_file,
            )
            .await;
            let (s, _s_provider) =
                agent(&runtime, "gate-on-s", supervised(scratch.path()), &s_file).await;
            let s_turn = start(&s);
            let s_request = parked(&s).await;
            tokio::time::timeout(std::time::Duration::from_secs(60), r.run("write"))
                .await
                .expect("r must not park")
                .expect("r's turn returns");
            assert!(r_file.exists(), "r's gated call ran without parking");
            assert!(r.approvals().pending().unwrap().is_empty());
            assert!(!s_file.exists(), "s is still parked");
            s.approvals()
                .decide(&s_request.request_id, ApprovalDecision::Deny)
                .expect("s decides");
            s_turn.await.unwrap();
            assert!(!s_file.exists());
        })
        .await
        .expect("test task");
    });
}
