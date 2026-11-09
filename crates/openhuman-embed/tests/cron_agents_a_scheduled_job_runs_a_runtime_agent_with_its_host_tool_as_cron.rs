mod common;
#[path = "support/cron_agents.rs"]
mod scenarios;

#[test]
fn a_scheduled_job_runs_a_runtime_agent_with_its_host_tool_as_cron() {
    scenarios::a_scheduled_job_runs_a_runtime_agent_with_its_host_tool_as_cron();
}
