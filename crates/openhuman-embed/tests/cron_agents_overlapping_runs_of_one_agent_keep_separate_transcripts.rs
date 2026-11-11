mod common;
#[path = "support/cron_agents.rs"]
mod scenarios;

#[test]
fn overlapping_runs_of_one_agent_keep_separate_transcripts() {
    scenarios::overlapping_runs_of_one_agent_keep_separate_transcripts();
}
