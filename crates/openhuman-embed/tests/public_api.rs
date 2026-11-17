use std::sync::Arc;

use openhuman_embed::{
    Access, Agent, AgentDefinitionSpec, AgentError, AgentSpec, AgentTurnOrigin, ApiKey,
    ApprovalDecision, Approvals, ApprovalsError, Core, CoreBuilder, CoreError, CoreRuntime, Cron,
    CronError, DomainSet, GroupMode, Harness, HostKind, JobRun, JobRunRecord, JobSchedule, JobSpec,
    JobTarget, PendingApproval, Provider, RemoveAgent, Runtime, RuntimeBuilder, RuntimeConfig,
    SandboxModeSpec, ScheduledJob, ServiceSet, SystemJobContext, ToolGroups, ToolScopeSpec,
    TrustedAccess, TrustedAutomationSource, Workspace, DEFAULT_MAX_AGENTS,
};

#[test]
fn exposes_the_host_facing_embedding_contract() {
    fn accepts_core(_: Core) {}
    fn accepts_builder(_: CoreBuilder) {}
    fn accepts_runtime(_: Arc<CoreRuntime>) {}
    fn accepts_harness(_: Harness) {}
    fn accepts_access(_: Access) {}
    fn accepts_provider(_: Provider) {}
    fn accepts_workspace(_: Workspace) {}
    fn applies_turn_origin(
        turn: openhuman_embed::Turn,
        origin: AgentTurnOrigin,
    ) -> openhuman_embed::Turn {
        turn.origin(origin)
    }
    fn accepts_runtime_handle(_: Runtime) {}
    fn accepts_runtime_builder(_: RuntimeBuilder) {}
    fn accepts_agent(_: Agent) {}
    fn accepts_agent_spec(_: AgentSpec) {}
    fn accepts_api_key(_: ApiKey) {}
    fn agent_approvals(agent: &Agent) -> Approvals {
        agent.approvals()
    }
    fn lists_pending(approvals: &Approvals) -> Result<Vec<PendingApproval>, ApprovalsError> {
        approvals.pending()
    }
    fn removes_agent<'a>(runtime: &'a Runtime, id: &str) -> RemoveAgent<'a> {
        runtime.remove_agent(id).purge()
    }
    fn steers_a_turn(
        turn: &mut openhuman_embed::Turn,
    ) -> Result<openhuman_embed::TurnSteering, openhuman_embed::SteerRefused> {
        let steering = turn.steering_handle();
        steering.steer("one more thing")?;
        Ok(steering)
    }
    fn hands_back_steers(
        steering: &openhuman_embed::TurnSteering,
        outcome: &openhuman_embed::TurnOutcome,
        refused: openhuman_embed::SteerRefused,
    ) -> Vec<String> {
        let mut left = outcome.unapplied_steers.clone();
        left.extend(steering.take_unapplied());
        left.push(refused.into_text());
        left
    }
    fn lifecycle_errors(error: &AgentError) -> Option<usize> {
        match error {
            AgentError::AgentLimit { limit } => Some(*limit),
            AgentError::UnknownId(_) => None,
            AgentError::Call(CoreError::AgentRemoved { .. }) => None,
            _ => None,
        }
    }

    let _ = accepts_core;
    let _ = steers_a_turn;
    let _ = hands_back_steers;
    let _ = accepts_builder;
    let _ = accepts_runtime;
    let _ = accepts_harness;
    let _ = accepts_access;
    let _ = accepts_provider;
    let _ = accepts_workspace;
    let _ = applies_turn_origin;
    let _ = accepts_runtime_handle;
    let _ = accepts_runtime_builder;
    let _ = accepts_agent;
    let _ = accepts_agent_spec;
    let _ = accepts_api_key;
    let _ = agent_approvals;
    let _ = lists_pending;
    let _ = removes_agent;
    let _ = lifecycle_errors;
    let _ = ApprovalDecision::ApproveOnce;
    assert_eq!(DEFAULT_MAX_AGENTS, 1024);
    let _ = Runtime::builder()
        .api_key("th_public_api")
        .backend_url("https://backend.example")
        .max_agents(DEFAULT_MAX_AGENTS);
    let _ = AgentSpec::new("public-api")
        .system_prompt("You are a test.")
        .definition(
            AgentDefinitionSpec::new()
                .tools(ToolScopeSpec::Named(vec!["read_file".into()]))
                .sandbox(SandboxModeSpec::ReadOnly),
        )
        .subagents([("public-api-helper", AgentDefinitionSpec::new())])
        .access(
            Access::readonly()
                .auto_approve(["read_file"])
                .auto_approve_all(false)
                .approval_gate(true),
        )
        .action_dir("/tmp/embed-public-api")
        .include_user_skills(false)
        .config(|_config| {});
    let _ = DomainSet::embedded;
    let _ = ServiceSet::none;
    let _ = HostKind::Library;
    let runtime_config = RuntimeConfig::default();
    let _ = CoreBuilder::new(HostKind::Library).config(runtime_config.clone());
    let _ = Harness::builder().config(runtime_config);
    let _ = ToolGroups::none().with("documents", GroupMode::Advertised);
    let automation = AgentTurnOrigin::TrustedAutomation {
        job_id: "embed-public-api".to_string(),
        source: TrustedAutomationSource::Cron,
    };
    let _ = Access::full()
        .trust("/tmp/embed-public-api", TrustedAccess::ReadWrite)
        .origin(automation);
}

#[test]
fn exposes_the_scheduling_contract() {
    fn cron_of(runtime: &Runtime) -> Cron<'_> {
        runtime.cron()
    }
    fn upserts(cron: &Cron<'_>, spec: JobSpec) -> Result<ScheduledJob, CronError> {
        cron.upsert(spec)
    }
    fn lists(cron: &Cron<'_>) -> Result<Vec<ScheduledJob>, CronError> {
        cron.list()
    }
    fn removes(cron: &Cron<'_>) -> Result<bool, CronError> {
        cron.remove("job")
    }
    fn history(cron: &Cron<'_>) -> Result<Vec<JobRunRecord>, CronError> {
        cron.runs("job", 10)
    }
    async fn runs_now(cron: &Cron<'_>) -> Result<JobRun, CronError> {
        cron.run_now("job").await
    }
    fn handles(runtime: &Runtime) -> Result<(), CronError> {
        runtime.on_system_job("digest", |ctx: SystemJobContext| async move {
            let _ = (ctx.job_id, ctx.name);
            Ok(())
        })
    }
    async fn controls_services(runtime: &Runtime) {
        runtime.start_services().await;
        runtime.stop_services();
    }
    let _ = (cron_of, upserts, lists, removes, history, handles);
    let _ = runs_now;
    let _ = controls_services;

    let spec = JobSpec::agent(
        "morning",
        "teeny",
        "Plan the day.",
        JobSchedule::Cron {
            expr: "0 8 * * *".into(),
            tz: None,
        },
    )
    .retries(0)
    .single_flight(true)
    .enabled(true);
    assert_eq!(spec.retries, Some(0));
    let system = JobSpec::system("digest", "digest", JobSchedule::Every { ms: 60_000 });
    assert_eq!(
        system.target,
        JobTarget::System {
            name: "digest".into()
        }
    );
    let _ = JobSchedule::At {
        at: std::time::SystemTime::now(),
    };
}

#[cfg(feature = "channels")]
#[test]
fn exposes_the_channels_contract() {
    use openhuman_embed::{
        ChannelError, ChannelListener, Channels, StreamMode, TelegramChannelSpec,
    };

    fn channels_of(runtime: &Runtime) -> Channels<'_> {
        runtime.channels()
    }
    fn starts_telegram(
        channels: &Channels<'_>,
        spec: TelegramChannelSpec,
    ) -> Result<ChannelListener, ChannelError> {
        channels.telegram(spec)
    }
    fn inspects(listener: &ChannelListener) -> (&str, &str, bool) {
        (
            listener.channel(),
            listener.agent_id(),
            listener.is_running(),
        )
    }
    fn stops(listener: ChannelListener) {
        listener.stop();
    }
    let _ = (channels_of, starts_telegram, inspects, stops);

    let spec = TelegramChannelSpec::new("123:abc", "teeny-chat")
        .allowed_users(["alice"])
        .allow_everyone()
        .mention_only(true)
        .stream_mode(StreamMode::default())
        .chat_id("-100");
    assert_eq!(spec.agent_id(), "teeny-chat");
    let _ = ChannelError::UnknownAgent("x".into());
    let _ = ChannelError::Invalid("x".into());
}

#[test]
fn exposes_the_curated_facades_and_compile_status() {
    use openhuman_embed::artifacts::{self, ArtifactKind, FileRoots};
    use openhuman_embed::chat_surface::{self, WebChannelEvent};
    use openhuman_embed::config::{self, RuntimeFlags};
    use openhuman_embed::identity::{self, UserIdentity};
    use openhuman_embed::{ControllerSchema, HTTP_SERVER_COMPILED_IN, VOICE_COMPILED_IN};

    // Signatures, pinned by coercion to a function pointer.
    let _: fn(&std::path::Path) -> Option<String> = config::read_active_user_id;
    let _: fn() -> anyhow::Result<std::path::PathBuf> = config::default_root_openhuman_dir;
    let _: fn() -> Option<UserIdentity> = identity::peek_credential_user_identity;
    let _: fn(&str) -> Option<ControllerSchema> = openhuman_embed::schema_for_rpc_method;
    let _: fn(WebChannelEvent) = chat_surface::publish_web_channel_event;
    let _ = chat_surface::subscribe_web_channel_events;
    let _ = chat_surface::register_approval_surface_subscriber;
    let _ = chat_surface::register_artifact_surface_subscriber;
    let _ = artifacts::resolve_ready_file;
    let _ = FileRoots::new("/tmp/embed-public-api").with_trusted([]);
    let _ = ArtifactKind::Document;
    let _ = RuntimeFlags {
        browser_allow_all: false,
        log_prompts: false,
    };

    const _: bool = HTTP_SERVER_COMPILED_IN;
    const _: bool = VOICE_COMPILED_IN;
}

#[cfg(feature = "modules")]
#[test]
fn exposes_the_module_configuration_facade() {
    let _: fn(std::path::PathBuf) -> Result<(), std::path::PathBuf> =
        openhuman_embed::modules::set_bundled_releases_dir;
    let _: &str = openhuman_embed::modules::browser::MODULE_ID;
}
