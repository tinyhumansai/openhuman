mod common;
#[path = "support/cron_agents.rs"]
mod scenarios;

#[test]
fn background_services_drive_the_scheduler_and_stop_with_the_runtime() {
    scenarios::background_services_drive_the_scheduler_and_stop_with_the_runtime();
}
