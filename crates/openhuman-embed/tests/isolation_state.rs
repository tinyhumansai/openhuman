//! Per-agent state on one workspace: two agents running the same session id
//! at the same time never see each other's history, each writes transcripts
//! under its own directory, and a skill one agent creates lands in its home
//! and never reaches a sibling's or the operator's home directory.

mod common;

use common::{
    chat_requests, offline_config, route, runtime, scripted_provider, stub_backend,
    tool_call_completion, tool_results,
};
use openhuman_embed::{
    Access, Agent, AgentDefinitionSpec, AgentSpec, Runtime, ToolScopeSpec, Workspace,
};
use serde_json::json;

const SESSION: &str = "same-session";

fn skill_name() -> String {
    format!("iso-state-skill-{}", std::process::id())
}

fn request_text(request: &wiremock::Request) -> String {
    String::from_utf8_lossy(&request.body).into_owned()
}

fn transcript_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

/// Whether `agent` has a root transcript for the test thread in its own store
/// under the workspace.
fn has_transcript(agent: &Agent) -> bool {
    tinyagents_session::port::WorkspaceStores::open(agent.workspace_dir(), agent.id())
        .expect("the agent's workspace store")
        .transcripts()
        .root_for_thread(SESSION)
        .is_some()
}

fn skill_tools() -> AgentDefinitionSpec {
    AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(vec!["create_skill".to_string()]))
}

async fn send(agent: &Agent, message: &str) {
    agent
        .turn(message)
        .session(SESSION)
        .send()
        .await
        .unwrap_or_else(|error| panic!("{}'s turn: {error}", agent.id()));
}

#[test]
fn agents_on_one_workspace_keep_their_own_state() {
    let _ = env_logger::builder().is_test(true).try_init();
    runtime().block_on(async {
        tokio::spawn(async move {
            let backend = stub_backend().await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .access(Access::full())
                .build()
                .await
                .expect("runtime builds");
            let name = skill_name();

            let a_provider = scripted_provider(
                vec![tool_call_completion(
                    "create_skill",
                    &json!({ "name": name, "description": "Made by alpha." }).to_string(),
                )],
                "alpha-done",
            )
            .await;
            let b_provider = scripted_provider(Vec::new(), "beta-done").await;
            let a = runtime
                .agent(
                    AgentSpec::new("state-a")
                        .provider(route(&a_provider, "a-model"))
                        .access(Access::full())
                        .definition(skill_tools()),
                )
                .expect("a instantiates");
            let b = runtime
                .agent(
                    AgentSpec::new("state-b")
                        .provider(route(&b_provider, "b-model"))
                        .access(Access::full())
                        .definition(skill_tools()),
                )
                .expect("b instantiates");
            assert_eq!(a.workspace_dir(), b.workspace_dir());

            // ── the same session id, concurrently, on both agents ──
            tokio::join!(send(&a, "alpha-secret-one"), send(&b, "beta-secret-one"));
            tokio::join!(send(&a, "alpha-secret-two"), send(&b, "beta-secret-two"));

            let a_requests = chat_requests(&a_provider).await;
            let b_requests = chat_requests(&b_provider).await;
            let a_last = request_text(a_requests.last().expect("a reached its provider"));
            let b_last = request_text(b_requests.last().expect("b reached its provider"));
            assert!(a_last.contains("alpha-secret-one") && a_last.contains("alpha-secret-two"));
            assert!(b_last.contains("beta-secret-one") && b_last.contains("beta-secret-two"));
            for request in &a_requests {
                assert!(
                    !request_text(request).contains("beta-secret"),
                    "a saw b's turn"
                );
            }
            for request in &b_requests {
                assert!(
                    !request_text(request).contains("alpha-secret"),
                    "b saw a's turn"
                );
            }

            // ── transcripts land in each agent's own store ──
            assert_ne!(a.transcripts_dir(), b.transcripts_dir());
            assert!(a.transcripts_dir().starts_with(a.home_dir()));
            assert!(b.transcripts_dir().starts_with(b.home_dir()));
            assert!(has_transcript(&a), "a's turns are stored under a");
            assert!(has_transcript(&b), "b's turns are stored under b");
            assert!(
                transcript_files(&a.workspace_dir().join("session_raw")).is_empty(),
                "nothing is written to the shared transcript directory"
            );

            // ── a's skill is a's alone ──
            let in_a = [
                a.home_dir().join("skills").join(&name),
                a.home_dir().join("workflows").join(&name),
            ];
            assert!(
                in_a.iter().any(|path| path.is_dir()),
                "create_skill wrote into a's home: {in_a:?}"
            );
            for root in ["skills", "workflows"] {
                assert!(!b.home_dir().join(root).join(&name).exists());
            }
            if let Some(home) = std::env::var_os("HOME") {
                let home = std::path::PathBuf::from(home).join(".openhuman");
                for root in ["skills", "workflows"] {
                    assert!(
                        !home.join(root).join(&name).exists(),
                        "the operator's home is untouched"
                    );
                }
            }
            let a_created = a_requests.iter().map(tool_results).collect::<String>();
            assert!(
                a_created.contains(&*a.home_dir().to_string_lossy()),
                "create_skill reported a location in a's home: {a_created}"
            );
        })
        .await
        .expect("test task");
    });
}
