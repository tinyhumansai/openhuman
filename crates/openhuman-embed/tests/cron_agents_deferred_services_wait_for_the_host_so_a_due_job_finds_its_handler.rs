mod common;
#[path = "support/cron_agents.rs"]
mod scenarios;

#[test]
fn deferred_services_wait_for_the_host_so_a_due_job_finds_its_handler() {
    scenarios::deferred_services_wait_for_the_host_so_a_due_job_finds_its_handler();
}
