//! End-to-end proof of the two-step library API: one `Runtime`, several
//! independently configured `Agent`s.
//!
//! One `#[test]` for the same reason as `harness_embed.rs`: a runtime claims a
//! process-wide slot. Everything is asserted against the one runtime this
//! process builds.
//!
//! No live LLM call is made. Each agent gets its own `wiremock` provider, which
//! is what makes the isolation assertions possible: a turn that leaked to the
//! other agent's provider would be recorded on the wrong mock.

mod common;

use common::{chat_completion, offline_config, runtime, stub_backend};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentError, AgentSpec, HostTurnTools, MemoryBinding, Provider,
    Runtime, SandboxModeSpec, ToolScopeSpec, Workspace,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, ResponseTemplate};

const API_KEY: &str = "th_test_key";

struct RecordingBudgetStopHook {
    inner: openhuman_core::agent::stop_hooks::BudgetStopHook,
    reason: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl openhuman_embed::seams::StopHook<openhuman_embed::seams::TurnCost>
    for RecordingBudgetStopHook
{
    fn name(&self) -> &str {
        "budget"
    }

    async fn check(
        &self,
        state: &openhuman_embed::seams::TurnState<'_>,
    ) -> openhuman_embed::seams::StopDecision {
        let decision = openhuman_embed::seams::StopHook::check(&self.inner, state).await;
        if let openhuman_embed::seams::StopDecision::Stop { reason } = &decision {
            *self.reason.lock().expect("stop reason lock") = Some(reason.clone());
        }
        decision
    }
}

/// A skills fixture with one bundle.
fn skills_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("skills fixture");
    let bundle = dir.path().join("alpha-skill");
    std::fs::create_dir_all(&bundle).expect("bundle dir");
    std::fs::write(
        bundle.join("SKILL.md"),
        "---\nname: alpha-skill\ndescription: A fixture skill.\n---\n# alpha-skill\n",
    )
    .expect("SKILL.md");
    dir
}

#[test]
fn one_runtime_hosts_independently_configured_agents() {
    let _ = env_logger::builder().is_test(true).try_init();

    let rt = runtime();
    rt.block_on(async {
        tokio::spawn(async move {
            let backend = wiremock::MockServer::start().await;
            // Managed inference for an agent with no route of its own: the
            // runtime's API key must arrive as a bearer, and nothing else.
            Mock::given(method("POST"))
                .and(path("/openai/v1/chat/completions"))
                .and(header(
                    "authorization",
                    format!("Bearer {API_KEY}").as_str(),
                ))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(chat_completion("managed-ok")),
                )
                .mount(&backend)
                .await;
            // Everything else the core might call incidentally.
            let _catch_all = stub_backend().await;
            Mock::given(wiremock::matchers::any())
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "success": true,
                    "data": { "id": "embed-test", "email": "local@openhuman.local" }
                })))
                .mount(&backend)
                .await;

            let provider_a = wiremock::MockServer::start().await;
            let provider_b = wiremock::MockServer::start().await;
            let budget_provider = wiremock::MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion("alpha-ok")))
                .mount(&provider_a)
                .await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion("beta-ok")))
                .mount(&provider_b)
                .await;
            let mut budget_completion = chat_completion("budget-metered");
            budget_completion["usage"]["prompt_tokens"] = serde_json::json!(1_000_000);
            budget_completion["usage"]["total_tokens"] = serde_json::json!(1_000_001);
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(budget_completion))
                .mount(&budget_provider)
                .await;
            #[allow(unused_variables)]
            let skills = skills_fixture();
            let beta_action = tempfile::tempdir().expect("beta action dir");

            let runtime = std::sync::Arc::new(
                Runtime::builder()
                    .config(offline_config())
                    .workspace(Workspace::Ephemeral)
                    .backend_url(backend.uri())
                    .api_key(API_KEY)
                    .build()
                    .await
                    .expect("runtime builds"),
            );
            assert!(runtime.has_api_key());

            // Regression: `has_api_key()` must read the credential store live,
            // not a snapshot taken when the runtime was built — a host that
            // clears or (re)stores the key on a running runtime via
            // `runtime.core().auth()` needs this to reflect that immediately.
            runtime
                .core()
                .auth()
                .clear_api_key()
                .await
                .expect("clear api key");
            assert!(
                !runtime.has_api_key(),
                "has_api_key() must observe a live clear, not a build-time snapshot"
            );
            runtime
                .core()
                .auth()
                .store_api_key(API_KEY)
                .await
                .expect("restore api key");
            assert!(
                runtime.has_api_key(),
                "has_api_key() must observe a live store, not a build-time snapshot"
            );

            let workspace_dir = runtime.workspace_dir().to_path_buf();
            let root_dir = runtime.root_dir().to_path_buf();
            assert!(workspace_dir.is_dir());
            assert!(
                std::env::var("OPENHUMAN_CORE_RPC_URL").is_err(),
                "a library runtime must not bind an RPC listener"
            );

            // The API key authenticates the runtime without a user.
            let state = runtime.core().auth().state().await.expect("auth state");
            assert!(state.is_authenticated, "api key must count as signed in");
            assert!(state.user_id.is_none(), "an api key carries no user");
            assert!(state.is_api_key(), "state names the credential: {state:?}");
            assert!(
                runtime
                    .core()
                    .auth()
                    .token()
                    .await
                    .expect("token")
                    .is_none(),
                "no session token exists"
            );

            // ── alpha: read-only, own provider, own skills, an MCP server ──
            #[allow(unused_mut)]
            let mut alpha_spec = AgentSpec::new("alpha")
                .provider(
                    Provider::openai_compatible(format!("{}/v1", provider_a.uri()), "sk-a")
                        .model("alpha-model"),
                )
                .access(Access::readonly())
                .definition(AgentDefinitionSpec::new().sandbox(SandboxModeSpec::ReadOnly));
            #[cfg(feature = "skills")]
            {
                alpha_spec = alpha_spec.skills_dir(skills.path());
            }
            #[cfg(feature = "mcp")]
            {
                alpha_spec = alpha_spec.mcp(openhuman_embed::McpServer::stdio(
                    "alpha-mcp",
                    "true",
                    Vec::<String>::new(),
                ));
            }
            let alpha = runtime.agent(alpha_spec).expect("alpha instantiates");

            // ── beta: full autonomy, own provider, caller-chosen action dir ──
            let beta = runtime
                .agent(
                    AgentSpec::new("beta")
                        .provider(
                            Provider::openai_compatible(format!("{}/v1", provider_b.uri()), "sk-b")
                                .model("beta-model"),
                        )
                        .access(Access::full())
                        .action_dir(beta_action.path()),
                )
                .expect("beta instantiates");

            // ── gamma: no route → managed inference on the runtime's key ──
            let gamma = runtime
                .agent(AgentSpec::new("gamma").access(Access::readonly()))
                .expect("gamma instantiates");

            // Per-turn withholding must survive the full embed path through
            // Agent::turn, session construction, and provider request
            // serialization. The same configured tool disappears for one
            // named session and is advertised again for the next.
            let withholding_provider = wiremock::MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion("turn-ok")))
                .mount(&withholding_provider)
                .await;
            let withholding_agent = runtime
                .agent(
                    AgentSpec::new("withholding")
                        .provider(
                            Provider::openai_compatible(
                                format!("{}/v1", withholding_provider.uri()),
                                "sk-withholding",
                            )
                            .model("withholding-model"),
                        )
                        .access(Access::readonly())
                        .definition(
                            AgentDefinitionSpec::new()
                                .tools(ToolScopeSpec::Named(vec!["shell".to_string()])),
                        )
                        .tools(|turn| HostTurnTools {
                            withheld: if turn.session_id() == Some("hidden-turn") {
                                std::collections::HashSet::from(["shell".to_string()])
                            } else {
                                std::collections::HashSet::new()
                            },
                            ..Default::default()
                        }),
                )
                .expect("withholding agent instantiates");

            let budget_agent = runtime
                .agent(
                    AgentSpec::new("budget")
                        .provider(
                            Provider::openai_compatible(
                                format!("{}/v1", budget_provider.uri()),
                                "sk-budget",
                            )
                            .model("gpt-5.5"),
                        )
                        .access(Access::full())
                        .definition(
                            AgentDefinitionSpec::new()
                                .tools(ToolScopeSpec::Named(vec!["shell".to_string()])),
                        ),
                )
                .expect("budget agent instantiates");

            withholding_agent
                .turn("hide the configured tool")
                .session("hidden-turn")
                .send()
                .await
                .expect("hidden turn runs");
            withholding_agent
                .turn("restore the configured tool")
                .session("visible-turn")
                .send()
                .await
                .expect("visible turn runs");

            let stop_reason = std::sync::Arc::new(std::sync::Mutex::new(None));
            let budgeted_turn = budget_agent
                .turn("report a metered response")
                .stop_hook(std::sync::Arc::new(RecordingBudgetStopHook {
                    inner: openhuman_core::agent::stop_hooks::BudgetStopHook::new(1.0),
                    reason: std::sync::Arc::clone(&stop_reason),
                }))
                .send()
                .await
                .expect("budgeted runtime-owned agent turn returns");
            assert_eq!(common::chat_requests(&budget_provider).await.len(), 1);
            assert_eq!(
                stop_reason.lock().expect("stop reason lock").as_deref(),
                Some("turn cost $5.0000 reached cap $1.0000")
            );
            let usage = budgeted_turn
                .usage
                .expect("runtime-owned agent reports turn usage");
            assert_eq!(usage.input_tokens, 1_000_000);
            assert!(
                usage.cost_usd.is_some_and(|usd| usd >= 1.0),
                "budgeted usage: {usage:?}"
            );
            let withholding_requests = common::chat_requests(&withholding_provider).await;
            assert_eq!(withholding_requests.len(), 2);
            let hidden_request: serde_json::Value =
                serde_json::from_slice(&withholding_requests[0].body).unwrap();
            let visible_request: serde_json::Value =
                serde_json::from_slice(&withholding_requests[1].body).unwrap();
            let has_shell = |request: &serde_json::Value| {
                request["tools"].as_array().is_some_and(|tools| {
                    tools.iter().any(|tool| tool["function"]["name"] == "shell")
                })
            };
            assert!(
                !has_shell(&hidden_request),
                "the hidden turn's provider request must omit shell"
            );
            assert!(
                has_shell(&visible_request),
                "the next turn's provider request must advertise shell again"
            );

            assert_eq!(
                runtime.agent_ids(),
                vec![
                    "alpha".to_string(),
                    "beta".to_string(),
                    "budget".to_string(),
                    "gamma".to_string(),
                    "withholding".to_string()
                ]
            );

            // Layout: every agent has its own home, transcripts and action dir.
            assert_eq!(alpha.home_dir(), workspace_dir.join("agents/alpha"));
            assert_eq!(
                alpha.transcripts_dir(),
                workspace_dir.join("agents/alpha/session_raw")
            );
            assert_eq!(alpha.action_dir(), root_dir.join("agents/alpha/action"));
            assert!(alpha.action_dir().is_dir());
            assert_eq!(beta.action_dir(), beta_action.path());
            assert!(
                !alpha.action_dir().starts_with(&workspace_dir),
                "action_dir must not sit inside the workspace"
            );
            #[cfg(feature = "skills")]
            {
                assert!(
                    alpha.skills_dir().join("alpha-skill/SKILL.md").is_file(),
                    "alpha's bundle lands in its agent-local skills root"
                );
                assert!(
                    !beta.skills_dir().join("alpha-skill").exists(),
                    "beta never sees alpha's skills"
                );
                assert!(
                    !workspace_dir.join("skills/alpha-skill").exists(),
                    "the shared workspace skills root stays untouched"
                );
            }
            #[cfg(feature = "mcp")]
            {
                assert_eq!(alpha.config().mcp_client.servers.len(), 1);
                assert!(beta.config().mcp_client.servers.is_empty());
            }
            assert_eq!(
                alpha.config().autonomy.level,
                openhuman_core::security::AutonomyLevel::ReadOnly
            );
            assert_eq!(
                beta.config().autonomy.level,
                openhuman_core::security::AutonomyLevel::Full
            );
            assert!(alpha.access().turn_origin().is_none());
            assert!(beta.access().turn_origin().is_some());

            // Turns land on each agent's own provider, and each agent keeps
            // only its own configured MCP servers (asserted above).
            let a0 = alpha.run("hello from alpha").await.expect("alpha turn");
            assert!(a0.reply.contains("alpha-ok"), "{:?}", a0.reply);
            assert_eq!(common::chat_requests(&provider_a).await.len(), 1);
            assert_eq!(common::chat_requests(&provider_b).await.len(), 0);
            let b0 = beta.run("hello from beta").await.expect("beta turn");
            assert!(b0.reply.contains("beta-ok"), "{:?}", b0.reply);
            assert_eq!(common::chat_requests(&provider_b).await.len(), 1);

            let a1 = alpha.run("hello from alpha").await.expect("alpha turn");
            assert!(a1.reply.contains("alpha-ok"), "{:?}", a1.reply);
            assert_eq!(common::chat_requests(&provider_a).await.len(), 2);
            assert_eq!(common::chat_requests(&provider_b).await.len(), 1);

            // The request bodies carry each agent's own model.
            let a_reqs = common::chat_requests(&provider_a).await;
            let b_reqs = common::chat_requests(&provider_b).await;
            let a_body: serde_json::Value = serde_json::from_slice(&a_reqs[0].body).unwrap();
            let b_body: serde_json::Value = serde_json::from_slice(&b_reqs[0].body).unwrap();
            assert_eq!(a_body["model"], "alpha-model");
            assert_eq!(b_body["model"], "beta-model");
            // Transcripts are keyed by agent and live in the agent's own store
            // under the workspace, and a thread resumes its own history.
            let listing = |agent_id: &str, session_id: &str| -> Vec<String> {
                tinyagents_session::port::WorkspaceStores::open(&workspace_dir, agent_id)
                    .expect("the agent's workspace store")
                    .transcripts()
                    .thread_transcripts(session_id, None)
                    .map(|found| found.roots.into_iter().map(|root| root.stem).collect())
                    .unwrap_or_default()
            };
            assert!(
                !workspace_dir.join("session_raw").exists()
                    || std::fs::read_dir(workspace_dir.join("session_raw"))
                        .map(|mut entries| entries.next().is_none())
                        .unwrap_or(true),
                "no agent writes to the shared transcript directory"
            );
            // Asserted against names the writer itself produced, not a shape
            // of our own. A turn's `session_id` is the raw session key the
            // runtime minted; TinyAgents derives the transcript stem from it
            // as `{key}-{digest}.{agent}-{digest}` (`session_stem`, with each
            // component carrying a disambiguating digest). The previous
            // `_alpha.jsonl` expectation predates that encoding — hosts used
            // to mint `{unix_ts}_{agent}` stems — so it can no longer match.
            for (agent, session_id) in [(&alpha, &a1.session_id), (&beta, &b0.session_id)] {
                let agent_id = agent.id();
                let transcripts = listing(agent_id, session_id);
                let transcript = transcripts
                    .iter()
                    .find(|f| f.starts_with(&format!("{session_id}-")))
                    .unwrap_or_else(|| {
                        panic!(
                            "{agent_id}'s transcript is derived from the session \
                             the runtime returned ({session_id}): {transcripts:?}"
                        )
                    });
                // ...and that transcript is scoped to its agent. The agent is
                // the last `.`-separated component of the stem. Asserting its
                // position and prefix rather than "contains the agent id":
                // a substring would also be satisfied by an agent id that
                // happened to appear in the session key, and would not fail if
                // the agent scope were dropped from the stem altogether.
                let component = transcript
                    .rsplit('.')
                    .next()
                    .expect("rsplit always yields at least one component");
                assert!(
                    component.starts_with(&format!("{agent_id}-")),
                    "{agent_id}'s transcript is scoped to its agent, so the final \
                     stem component is `{agent_id}-<digest>`: {transcript}"
                );
            }

            let a2 = alpha
                .turn("and again")
                .session(&a1.session_id)
                .send()
                .await
                .expect("alpha second turn");
            assert_eq!(a2.session_id, a1.session_id);
            let a2_req = common::chat_requests(&provider_a).await.remove(2);
            let a2_body: serde_json::Value = serde_json::from_slice(&a2_req.body).unwrap();
            let a2_messages = a2_body["messages"].as_array().cloned().unwrap_or_default();
            assert!(
                a2_messages.iter().any(|m| m["content"]
                    .as_str()
                    .is_some_and(|c| c.contains("hello from alpha"))),
                "the second turn on a thread must carry the first exchange: {a2_messages:?}"
            );

            // Managed inference authenticates with the runtime's API key.
            let g1 = gamma.run("hello from gamma").await.expect("gamma turn");
            assert!(g1.reply.contains("managed-ok"), "{:?}", g1.reply);
            let managed: Vec<_> = backend
                .received_requests()
                .await
                .unwrap()
                .into_iter()
                .filter(|r| r.url.path() == "/openai/v1/chat/completions")
                .collect();
            assert_eq!(managed.len(), 1, "one managed completion");
            assert_eq!(
                managed[0]
                    .headers
                    .get("authorization")
                    .map(|v| v.to_str().unwrap()),
                Some(format!("Bearer {API_KEY}").as_str())
            );
            assert!(
                managed[0].headers.get("x-api-key").is_none(),
                "inference sends the key as a bearer only"
            );

            // Interleaved concurrency across agents keeps their providers apart.
            let mut turns = tokio::task::JoinSet::new();
            for index in 0..25 {
                let alpha = alpha.clone();
                let beta = beta.clone();
                turns.spawn(async move { alpha.turn(format!("a{index}")).send().await });
                turns.spawn(async move { beta.turn(format!("b{index}")).send().await });
            }
            let mut a_count = 0;
            let mut b_count = 0;
            while let Some(outcome) = turns.join_next().await {
                let outcome = outcome.expect("no panic").expect("turn runs");
                if outcome.reply.contains("alpha-ok") {
                    a_count += 1;
                } else if outcome.reply.contains("beta-ok") {
                    b_count += 1;
                } else {
                    panic!("unexpected reply {:?}", outcome.reply);
                }
            }
            assert_eq!((a_count, b_count), (25, 25));
            assert_eq!(common::chat_requests(&provider_a).await.len(), 3 + 25);
            assert_eq!(common::chat_requests(&provider_b).await.len(), 1 + 25);

            // Ids are unique while alive, and validated.
            let err = runtime
                .agent(AgentSpec::new("alpha"))
                .expect_err("duplicate id");
            assert!(
                matches!(err, AgentError::DuplicateId(ref id) if id == "alpha"),
                "{err:?}"
            );
            let err = runtime
                .agent(AgentSpec::new("Not Valid!"))
                .expect_err("invalid id");
            assert!(matches!(err, AgentError::InvalidId { .. }), "{err:?}");
            let err = runtime
                .agent(AgentSpec::new("wide").domains(openhuman_embed::DomainSet::full()))
                .expect_err("widening the runtime");
            assert!(matches!(err, AgentError::WidensRuntime(_)), "{err:?}");

            // A memory binding gives an agent memory of its own: its turns
            // are logged under the bound id, below the bound root. An
            // unbound agent keeps the runtime's defaults.
            let bound = runtime
                .agent(
                    AgentSpec::new("employee")
                        .memory(MemoryBinding::new("employee-7").root("team:acme")),
                )
                .expect("a bound agent instantiates");
            assert_eq!(
                bound.config().memory.agent_id.as_deref(),
                Some("employee-7")
            );
            assert_eq!(bound.config().memory.root.as_deref(), Some("team:acme"));
            assert_eq!(beta.config().memory.agent_id, None);
            let err = runtime
                .agent(
                    AgentSpec::new("misbound")
                        .memory(MemoryBinding::new("x").root("not a namespace")),
                )
                .expect_err("an invalid memory root");
            assert!(matches!(err, AgentError::Invalid(_)), "{err:?}");
            let err = runtime
                .agent(AgentSpec::new("unnamed").memory(MemoryBinding::new("  ")))
                .expect_err("a blank memory agent id");
            assert!(matches!(err, AgentError::Invalid(_)), "{err:?}");
            drop(bound);

            // Dropping every handle releases the id.
            drop(withholding_agent);
            drop(gamma);
            drop(budget_agent);
            assert_eq!(
                runtime.agent_ids(),
                vec!["alpha".to_string(), "beta".to_string()]
            );
            let _gamma_again = runtime
                .agent(AgentSpec::new("gamma"))
                .expect("id reusable after drop");

            drop(alpha);
            drop(beta);
            drop(_gamma_again);
            let Ok(runtime) = std::sync::Arc::try_unwrap(runtime) else {
                panic!("the test is the runtime's sole owner once every agent is dropped");
            };
            drop(runtime);
            assert!(
                !workspace_dir.exists(),
                "an ephemeral workspace must be removed with its runtime"
            );
        })
        .await
        .expect("library host task did not panic");
    });
}
