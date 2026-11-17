---
description: "Cron provides durable named jobs, explicit run-now execution and job history addressed to registered agents."
---

# Cron

The typed `Cron` facade creates named jobs with `JobSpec` and `JobSchedule`. `upsert` preserves a job's identity when the same name is configured again; `runs` exposes execution history. An agent-targeted job resolves the configured agent ID and executes with that agent's provider, prompt, tools and derived context.

Background scheduling is a runtime service choice. Explicit `run_now` is useful for tests and host-triggered execution without relying on wall-clock scheduling. Keep the agent registered while scheduled work should be available.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Cron storage, scheduler service and system-job handlers | Addressed job target and the agent configuration used for its turn |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/cron.rs#cron -->

```rust
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
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/cron.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example checks stable upsert identity, executes a job immediately, inspects a successful run record, and removes the job. No test waits for a daily schedule to arrive.

Removing or dropping an agent deregisters its context, leaving addressed cron jobs dormant rather than silently running them as the operator. Hosts can register runtime system-job handlers for work they own outside a model turn. Review [runtime configuration](runtime-defaults.md) and [channels](channels.md) when mixing scheduled work with public chat input.
