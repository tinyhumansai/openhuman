use super::*;

/// Run an agent job under a fresh run id for focused scheduler tests.
pub(crate) async fn run_agent_job(
    config: &Config,
    job: &CronJob,
) -> (bool, String, Option<String>) {
    let run_id = uuid::Uuid::new_v4().to_string();
    run_agent_job_for_run(
        &crate::core::runtime::DispatchCtx::host_default(),
        config,
        job,
        &run_id,
    )
    .await
}

mod corrective_turn {
    use super::*;
    use crate::agent::host_agents::{self, HostAgent, HostAgentResolver};
    use crate::core::runtime::{CoreContext, DomainSet};
    use std::sync::Arc;
    use tinyflows_bus::schedule::types::{DeliveryConfig, JobType, Schedule};

    const AGENT_ID: &str = "cron-no-reply-agent";

    struct OneAgent(Config);

    impl HostAgentResolver for OneAgent {
        fn resolve(&self, agent_id: &str) -> Option<HostAgent> {
            if agent_id != AGENT_ID {
                return None;
            }
            let mut definition =
                crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
                    .get("orchestrator")
                    .cloned()
                    .unwrap();
            definition.id = agent_id.to_string();
            definition.subagents.clear();
            definition.system_prompt =
                tinyagents_definition::PromptSource::Inline("CRON_NO_REPLY_AGENT".into());
            definition.tools = tinyagents_definition::ToolScope::Named(Vec::new());
            let context = CoreContext::for_test_with_host_kind_and_config(
                crate::core::types::HostKind::Library,
                DomainSet::full(),
                self.0.clone(),
            );
            Some(HostAgent {
                definition,
                config: self.0.clone(),
                host_tools: None,
                hooks: Default::default(),
                context,
            })
        }
    }

    struct ScriptedModel {
        replies: std::sync::Mutex<std::collections::VecDeque<&'static str>>,
        requests: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    }

    #[async_trait::async_trait]
    impl tinyinference_llm::model::ChatModel<()> for ScriptedModel {
        async fn invoke(
            &self,
            _state: &(),
            request: tinyinference_llm::model::ModelRequest,
        ) -> tinyinference_llm::Result<tinyinference_llm::model::ModelResponse> {
            self.requests
                .lock()
                .unwrap()
                .push(serde_json::to_value(&request).unwrap());
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or("unexpected");
            Ok(tinyinference_llm::model::ModelResponse::assistant(reply))
        }
    }

    fn config(tmp: &tempfile::TempDir) -> Config {
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut config = Config {
            workspace_dir: workspace.clone(),
            action_dir: workspace,
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        config.local_ai.runtime_enabled = false;
        config.memory.conversations.enabled = false;
        config.agent.session_dual_write = false;
        config.agent.session_shadow_reads = false;
        config.default_model = Some("fixture".into());
        config.secrets.encrypt = false;
        config.modules.source = crate::config::schema::ModuleSourceMode::Source;
        config
    }

    fn reminder_job() -> CronJob {
        CronJob {
            claim_generation: 0,
            policy: Default::default(),
            id: format!("job-{}", uuid::Uuid::new_v4()),
            expression: "*/5 * * * *".into(),
            schedule: Schedule::Every { every_ms: 300_000 },
            command: String::new(),
            prompt: Some("Remind the user to drink water".into()),
            name: Some("water".into()),
            job_type: JobType::Agent,
            session_target: SessionTarget::Isolated,
            model: None,
            agent_id: Some(AGENT_ID.into()),
            enabled: true,
            delivery: DeliveryConfig {
                mode: "origin".into(),
                channel: None,
                to: None,
                best_effort: true,
            },
            delete_after_run: false,
            created_at: chrono::Utc::now(),
            next_run: chrono::Utc::now(),
            last_run: None,
            last_status: None,
            last_output: None,
            origin: None,
        }
    }

    fn last_user_message(request: &serde_json::Value) -> String {
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find_map(|message| message.get("user"))
            .map(|user| user.to_string())
            .unwrap_or_default()
    }

    async fn run(
        job: CronJob,
        first: &'static str,
        then: &'static str,
    ) -> ((bool, String), Vec<serde_json::Value>) {
        let tmp = tempfile::tempdir().unwrap();
        crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()
            .unwrap();
        let config = config(&tmp);
        #[cfg(feature = "security-module")]
        crate::modules::security::ensure_loaded(&config)
            .await
            .expect("the local TinySecurity module should load for scheduler agent tests");
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        crate::agent::session_host::test_support::install_scripted_model(Arc::new(ScriptedModel {
            replies: std::sync::Mutex::new([first, then].into()),
            requests: Arc::clone(&requests),
        }));
        let resolver: Arc<dyn HostAgentResolver> = Arc::new(OneAgent(config.clone()));
        host_agents::install(Arc::clone(&resolver));
        let (success, output, error) = run_agent_job(&config, &job).await;
        host_agents::clear_if(&resolver);
        assert!(error.is_none(), "{error:?}");
        let requests = requests.lock().unwrap().clone();
        ((success, output), requests)
    }

    fn on_agent_runtime(test: impl std::future::Future<Output = ()> + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(test);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn a_reminder_that_answers_no_reply_gets_one_corrective_turn() {
        let _slot = host_agents::tests::lock();
        on_agent_runtime(async {
            let (outcome, chats) = run(reminder_job(), "NO_REPLY", "Time to drink water!").await;
            assert_eq!(outcome, (true, "Time to drink water!".to_string()));
            assert_eq!(chats.len(), 2, "{chats:#?}");
            assert!(!last_user_message(&chats[0]).contains(NO_REPLY_CORRECTION));
            let corrective = last_user_message(&chats[1]);
            assert!(corrective.contains(NO_REPLY_CORRECTION), "{corrective}");
            assert!(!corrective.contains("drink water"), "{corrective}");
            assert!(last_user_message(&chats[0]).contains("drink water"));
        });
    }

    #[test]
    fn a_reminder_that_keeps_answering_no_reply_delivers_the_fallback() {
        let _slot = host_agents::tests::lock();
        on_agent_runtime(async {
            let (outcome, chats) = run(reminder_job(), "NO_REPLY", "No_reply.").await;
            assert_eq!(outcome, (true, "Reminder: water".to_string()));
            assert_eq!(chats.len(), 2, "{chats:#?}");
        });
    }

    #[test]
    fn a_conditional_job_may_answer_no_reply_without_correction() {
        let _slot = host_agents::tests::lock();
        on_agent_runtime(async {
            let mut job = reminder_job();
            job.prompt = Some("Check the inbox; reply NO_REPLY if nothing changed".into());
            let (outcome, chats) = run(job, "NO_REPLY", "unexpected").await;
            assert_eq!(outcome, (true, "NO_REPLY".to_string()));
            assert_eq!(chats.len(), 1, "{chats:#?}");
        });
    }
}
