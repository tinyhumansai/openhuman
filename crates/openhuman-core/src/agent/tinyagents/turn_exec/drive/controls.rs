use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    max_iterations: usize,
    pause_at_cap: bool,
    early_exit_tools: &[&str],
    subagent_scope: Option<SubagentScope>,
    message_run_queue: Option<tinyagents_harness::run_queue::RunQueueHandle>,
    run_queue: Option<Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    is_subagent: bool,
    run_context: &OpenHumanRunContext,
) -> TurnControls {
    let steer_thread_label =
        steering_forwarder::thread_label(subagent_scope.as_ref(), run_context.origin.as_ref());
    let controls = TurnControls {
        max_iterations: Some(max_iterations),
        pause_at_cap,
        early_exit_tools: early_exit_tools
            .iter()
            .map(|name| (*name).to_string())
            .collect::<BTreeSet<_>>(),
        // The host emits the terminal `TurnCompleted` itself (it carries the
        // stop classification), so the driver never fires a completion sink.
        defer_turn_completed: true,
        scope: match &subagent_scope {
            Some(scope) => TurnScope::Subagent(DriverSubagentScope {
                agent_id: scope.agent_id.clone(),
                task_id: scope.task_id.clone(),
            }),
            None => TurnScope::Root,
        },
        // Forwards mid-flight steers from the session's run queue, drains
        // residual (delivered-but-unapplied) steers back into it when the turn
        // ends — also on cancellation (issue #4456).
        steering: message_run_queue
            .map(steering_forwarder::message_queue_feed)
            .or_else(|| run_queue.map(steering_forwarder::queue_feed)),
        registrar: is_subagent
            .then(|| Arc::new(steering_forwarder::SharedSteeringRegistrar) as Arc<_>),
        thread_label: Some(steer_thread_label),
        // Only the top-level turn's cap pause is binding on dispatch; a
        // delegated child reaching its own cap is routine and must not stop the
        // parent's fan-out (it still gets its advisory `Pause`). `Pause` is
        // advisory, so the pause is also recorded on the turn's dispatch guard
        // to stop a racing sub-agent dispatch (#5804).
        cap_pause_recorder: (!is_subagent)
            .then(|| run_context.dispatch.clone())
            .flatten()
            .map(|guard| {
                Arc::new(move |completed: u64, cap: u64| {
                    guard.record_pause_requested(completed, cap)
                }) as tinyagents_harness::events::PauseRecorder
            }),
        ..TurnControls::default()
    };
    controls
}
