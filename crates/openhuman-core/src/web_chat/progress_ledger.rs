//! Host adapter for the owner-provided generic progress ledger recorder.

pub(super) use tinyagents_orchestration::run_progress_ledger::ProgressLedgerContext;
pub(super) type Ledger = tinyagents_orchestration::run_progress_ledger::RunProgressLedger;

pub(super) fn for_turn(
    workspace_dir: &std::path::Path,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
) -> Ledger {
    let store = crate::storage::sessions::ledger_for(workspace_dir);
    Ledger::new(
        store,
        tinyagents_orchestration::run_progress_ledger::RunProgressIdentity {
            client_id: client_id.to_owned(),
            thread_id: thread_id.to_owned(),
            request_id: request_id.to_owned(),
            source: "web_channel".to_owned(),
            agent_id: "orchestrator".to_owned(),
        },
    )
}
