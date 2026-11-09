use super::*;

/// Deliver under a fresh run id and discard the delivery status.
pub(crate) async fn deliver_if_configured(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    output: &str,
    success: bool,
) -> Result<()> {
    let run_id = uuid::Uuid::new_v4().to_string();
    deliver_run(ctx, config, job, &run_id, output, success)
        .await
        .map(|_| ())
}

fn proactive_job() -> CronJob {
    CronJob {
        claim_generation: 0,
        policy: Default::default(),
        id: format!("test-job-{}", uuid::Uuid::new_v4()),
        expression: "* * * * *".into(),
        schedule: tinyflows_bus::schedule::types::Schedule::Every { every_ms: 300_000 },
        command: String::new(),
        prompt: None,
        name: None,
        job_type: JobType::Agent,
        session_target: tinyflows_bus::schedule::types::SessionTarget::Isolated,
        model: None,
        agent_id: None,
        enabled: true,
        delivery: DeliveryConfig {
            mode: "proactive".into(),
            channel: None,
            to: None,
            best_effort: true,
        },
        delete_after_run: false,
        created_at: Utc::now(),
        next_run: Utc::now(),
        last_run: None,
        last_status: None,
        last_output: None,
        origin: None,
    }
}

async fn test_config(tmp: &tempfile::TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

async fn cron_alerts(config: &Config) -> usize {
    crate::desktop::notifications::store::list(config, 10, 0, Some("cron"), None)
        .unwrap()
        .len()
}

#[derive(Default)]
struct ProactiveCollector {
    source: String,
    messages: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl tinybus::EventHandler<crate::core::events::DomainEvent> for ProactiveCollector {
    fn name(&self) -> &str {
        "test::scheduler::proactive_no_reply::collector"
    }

    async fn handle(&self, event: &crate::core::events::DomainEvent) {
        if let crate::core::events::DomainEvent::ProactiveMessageRequested {
            source, message, ..
        } = event
        {
            if *source == self.source {
                self.messages.lock().unwrap().push(message.clone());
            }
        }
    }
}

async fn proactive_run(
    job: &CronJob,
    config: &Config,
    output: &str,
) -> (DeliveryStatus, Vec<String>) {
    crate::core::bus::init().await.expect("bus init");
    let messages = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let _handle = crate::core::bus::BUS
        .subscribe(std::sync::Arc::new(ProactiveCollector {
            source: format!("cron:{}", job.id),
            messages: std::sync::Arc::clone(&messages),
        }))
        .expect("bus subscriber installed");
    let status = deliver_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        job,
        "run-1",
        output,
        true,
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let seen = messages.lock().unwrap().clone();
    (status, seen)
}

fn proactive_agent_job(prompt: &str) -> CronJob {
    let mut job = proactive_job();
    job.prompt = Some(prompt.into());
    job.name = Some("stretch".into());
    job
}

#[tokio::test]
async fn proactive_allowed_no_reply_publishes_nothing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = proactive_agent_job("Summarise new mail, otherwise reply NO_REPLY");
    let (status, seen) = proactive_run(&job, &config, "NO_REPLY").await;
    assert_eq!(status, DeliveryStatus::Suppressed);
    assert!(seen.is_empty(), "{seen:?}");
    assert_eq!(cron_alerts(&config).await, 0);
}

#[tokio::test]
async fn proactive_disallowed_no_reply_publishes_the_fallback_not_the_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = proactive_agent_job("Remind the user to stretch");
    let (status, seen) = proactive_run(&job, &config, "`NO_REPLY`").await;
    assert_eq!(status, DeliveryStatus::Delivered);
    assert_eq!(seen, vec!["Reminder: stretch".to_string()]);
}
