//! Title: Run and inspect an agent cron job
//! Summary: Run and inspect an agent cron job.
//! Run: offline; optional live via OPENHUMAN_EXAMPLE_LIVE=1 and BASE_URL/API_KEY/MODEL.
//! Feature: default

mod support;
use openhuman_embed::{AgentSpec, Runtime, Workspace};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let provider = support::provider("hello from the stub").await;
    let runtime = Runtime::builder()
        .config(support::offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .provider(support::example_provider(&provider)?)
        .build()
        .await?;
    // ANCHOR: cron
    let agent = runtime.agent(AgentSpec::new("scheduled"))?;
    let cron = runtime.cron();
    let spec = openhuman_embed::JobSpec::agent(
        "daily",
        agent.id(),
        "Say hello",
        openhuman_embed::JobSchedule::Cron {
            expr: "0 0 * * *".into(),
            tz: None,
        },
    );
    let first = cron.upsert(spec.clone())?;
    assert_eq!(cron.upsert(spec)?.id, first.id);
    let run = cron.run_now("daily").await?;
    assert!(run.success, "{}", run.output);
    if support::offline() {
        assert!(run.output.contains("hello from the stub"));
    }
    assert_eq!(cron.runs("daily", 1)?[0].status, "ok");
    assert!(cron.remove("daily")?);
    assert!(cron.list()?.is_empty());
    println!("cron execution and history verified");
    // ANCHOR_END: cron
    support::passed("cron");
    Ok(())
}
