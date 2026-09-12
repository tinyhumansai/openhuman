use super::*;
use std::sync::Arc;
use tinyagents_harness::ids::{CallId, RunId, ThreadId};
use tinyinference::usage::Usage;
use tokio::sync::mpsc;

fn run() -> RunId {
    RunId::new("run-1")
}

fn started() -> ProgressEvent {
    ProgressEvent::Started {
        run: run(),
        thread: Some(ThreadId::new("thread-1")),
        agent: "orchestrator".to_string(),
    }
}

fn tool_call(name: &str) -> ProgressEvent {
    ProgressEvent::ToolCall {
        run: run(),
        call: CallId::new("call-1"),
        tool: name.to_string(),
    }
}

fn tool_call_finished(success: bool, output: &str) -> ProgressEvent {
    ProgressEvent::ToolCallFinished {
        run: run(),
        call: CallId::new("call-1"),
        success,
        output: output.to_string(),
    }
}

fn sink(capacity: usize) -> (OpenHumanProgressSink, mpsc::Receiver<AgentProgress>) {
    let (tx, rx) = mpsc::channel(capacity);
    (OpenHumanProgressSink::new(tx), rx)
}

/// Drains and returns the first `ToolCallCompleted`, panicking if none
/// arrives — a silently-absent completion is exactly the #88 bug.
fn drain_completion(
    rx: &mut mpsc::Receiver<AgentProgress>,
) -> (String, bool, usize, String, u64, u32, bool) {
    while let Ok(ev) = rx.try_recv() {
        if let AgentProgress::ToolCallCompleted {
            tool_name,
            success,
            output_chars,
            output,
            elapsed_ms,
            iteration,
            failure,
            ..
        } = ev
        {
            return (
                tool_name,
                success,
                output_chars,
                output,
                elapsed_ms,
                iteration,
                failure.is_some(),
            );
        }
    }
    panic!("no ToolCallCompleted was forwarded");
}

#[tokio::test]
async fn a_finished_tool_call_closes_the_row_it_opened() {
    // The #88 payoff: a row opened by `ToolCall` must be closable. Before
    // the crate carried a completion milestone this could not be emitted at
    // all, and every timeline entry stayed `running` forever.
    let (sink, mut rx) = sink(16);
    sink.emit(tool_call("search")).await;
    sink.emit(tool_call_finished(true, "results")).await;

    let (tool_name, success, output_chars, output, _elapsed, iteration, has_failure) =
        drain_completion(&mut rx);
    // The tool name does not travel on the closing event; it is recovered
    // from the opening one.
    assert_eq!(tool_name, "search");
    assert!(success);
    assert_eq!(output, "results");
    assert_eq!(output_chars, "results".chars().count());
    assert_eq!(iteration, 1);
    assert!(!has_failure, "a success carries no failure classification");
}

#[tokio::test]
async fn a_failed_tool_reports_failure_and_carries_a_classification() {
    // The specific corruption #88 warned about: defaulting `success: true`
    // would mark a failed tool successful in both the timeline and the
    // trace exporter.
    let (sink, mut rx) = sink(16);
    sink.emit(tool_call("search")).await;
    sink.emit(tool_call_finished(false, "connection refused"))
        .await;

    let (_tool, success, _chars, output, _elapsed, _iter, has_failure) = drain_completion(&mut rx);
    assert!(!success, "a failed tool must not report success");
    assert_eq!(output, "connection refused");
    assert!(
        has_failure,
        "a failure must carry a classification for the timeline"
    );
}

#[tokio::test]
async fn a_completion_is_filed_under_the_round_its_call_opened_in() {
    // Model output between a call and its result advances the round, so
    // reading the live counter at completion would file the tool under the
    // wrong iteration.
    let (sink, mut rx) = sink(16);
    sink.emit(tool_call("search")).await;
    sink.emit(ProgressEvent::Token {
        run: run(),
        text: "thinking".to_string(),
    })
    .await;
    sink.emit(tool_call_finished(true, "ok")).await;

    let (_tool, _success, _chars, _output, _elapsed, iteration, _has_failure) =
        drain_completion(&mut rx);
    assert_eq!(iteration, 1, "the call opened in round 1");
}

#[tokio::test]
async fn an_unmatched_completion_is_dropped_rather_than_invented() {
    // With no opening record there is no honest tool name or duration. A
    // missing row is recoverable; a fabricated one is not.
    let (sink, mut rx) = sink(16);
    sink.emit(started()).await;
    let _ = rx.try_recv();
    sink.emit(tool_call_finished(true, "orphan")).await;

    assert!(
        rx.try_recv().is_err(),
        "a completion with no open call must not be forwarded"
    );
}

#[tokio::test]
async fn a_completion_is_emitted_only_once_per_call() {
    // The open-call record is removed on close, so a duplicate close (a
    // retry, or a re-delivered event) cannot produce a second row.
    let (sink, mut rx) = sink(16);
    sink.emit(tool_call("search")).await;
    sink.emit(tool_call_finished(true, "ok")).await;
    let _ = drain_completion(&mut rx);

    sink.emit(tool_call_finished(true, "ok")).await;
    assert!(
        rx.try_recv().is_err(),
        "a duplicate completion must not open a second row"
    );
}

#[tokio::test]
async fn started_maps_to_turn_started() {
    let (sink, mut rx) = sink(8);
    sink.emit(started()).await;
    assert!(matches!(
        rx.try_recv().expect("event forwarded"),
        AgentProgress::TurnStarted
    ));
}

#[tokio::test]
async fn tool_call_maps_to_tool_call_started_with_null_arguments() {
    let (sink, mut rx) = sink(8);
    sink.emit(tool_call("search")).await;

    match rx.try_recv().expect("event forwarded") {
        AgentProgress::ToolCallStarted {
            call_id,
            tool_name,
            arguments,
            iteration,
            display_label,
            display_detail,
        } => {
            assert_eq!(call_id, "call-1");
            assert_eq!(tool_name, "search");
            // The crate keeps tool arguments off the progress side channel;
            // a non-null value here would mean the adapter invented one.
            assert!(arguments.is_null());
            assert_eq!(iteration, 1, "iterations are 1-based");
            assert!(display_label.is_none());
            assert!(display_detail.is_none());
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn token_maps_to_text_delta_on_the_current_round() {
    let (sink, mut rx) = sink(8);
    sink.emit(ProgressEvent::Token {
        run: run(),
        text: "hello".to_string(),
    })
    .await;

    match rx.try_recv().expect("event forwarded") {
        AgentProgress::TextDelta { delta, iteration } => {
            assert_eq!(delta, "hello");
            assert_eq!(iteration, 1);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn parallel_tool_calls_share_one_iteration() {
    // A model that requests two tools in one response produces two
    // consecutive `ToolCall` events belonging to the *same* LLM iteration.
    // Counting one per call reported this turn as three iterations and
    // mislabelled the second tool as iteration 2.
    let (sink, mut rx) = sink(16);
    sink.emit(tool_call("a")).await;
    sink.emit(tool_call("b")).await;
    sink.emit(ProgressEvent::Token {
        run: run(),
        text: "x".to_string(),
    })
    .await;

    let iterations: Vec<u32> = std::iter::from_fn(|| rx.try_recv().ok())
        .map(|ev| match ev {
            AgentProgress::ToolCallStarted { iteration, .. } => iteration,
            AgentProgress::TextDelta { iteration, .. } => iteration,
            other => panic!("unexpected event: {other:?}"),
        })
        .collect();
    // Both calls are iteration 1; the model's next reply is iteration 2.
    assert_eq!(iterations, vec![1, 1, 2]);
}

/// The failure that matters most on a shared sink: a sub-run finishing must
/// not tell the bridge the whole request is done, or the turn is closed out
/// while other runs are still emitting.
#[tokio::test]
async fn a_sub_runs_lifecycle_is_not_the_requests_lifecycle() {
    let (sink, mut rx) = sink(16);
    let root = RunId::new("root");
    let child = RunId::new("child");

    sink.emit(ProgressEvent::Started {
        run: root.clone(),
        thread: None,
        agent: "orchestrator".to_string(),
    })
    .await;
    sink.emit(ProgressEvent::Started {
        run: child.clone(),
        thread: None,
        agent: "worker".to_string(),
    })
    .await;
    sink.emit(ProgressEvent::Finished {
        run: child,
        usage: None,
    })
    .await;

    let events: Vec<AgentProgress> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(
        events.len(),
        1,
        "only the root's start should surface, got {events:?}"
    );
    assert!(matches!(events[0], AgentProgress::TurnStarted));

    // The root's own completion still lands.
    sink.emit(ProgressEvent::Finished {
        run: root,
        usage: None,
    })
    .await;
    assert!(matches!(
        rx.try_recv().expect("root completion"),
        AgentProgress::TurnCompleted { .. }
    ));
}

/// A child's tool calls must not renumber the parent's iterations.
#[tokio::test]
async fn iteration_counters_are_scoped_per_run() {
    let (sink, mut rx) = sink(16);
    let root = RunId::new("root");

    sink.emit(ProgressEvent::Started {
        run: root.clone(),
        thread: None,
        agent: "orchestrator".to_string(),
    })
    .await;
    let _ = rx.try_recv();

    // Child does three rounds of work.
    for i in 0..3 {
        sink.emit(ProgressEvent::ToolCall {
            run: RunId::new("child"),
            call: CallId::new(format!("c{i}")),
            tool: "grep".to_string(),
        })
        .await;
        sink.emit(ProgressEvent::Token {
            run: RunId::new("child"),
            text: "x".to_string(),
        })
        .await;
    }
    while rx.try_recv().is_ok() {}

    // The parent's first tool call is still its first iteration.
    sink.emit(ProgressEvent::ToolCall {
        run: root,
        call: CallId::new("parent-1"),
        tool: "shell".to_string(),
    })
    .await;

    match rx.try_recv().expect("parent tool call") {
        AgentProgress::ToolCallStarted { iteration, .. } => assert_eq!(
            iteration, 1,
            "the child's work must not advance the parent's iteration"
        ),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn a_tool_call_after_model_output_opens_a_new_iteration() {
    let (sink, mut rx) = sink(16);
    sink.emit(tool_call("a")).await;
    sink.emit(ProgressEvent::Token {
        run: run(),
        text: "thinking".to_string(),
    })
    .await;
    sink.emit(tool_call("b")).await;

    let iterations: Vec<u32> = std::iter::from_fn(|| rx.try_recv().ok())
        .map(|ev| match ev {
            AgentProgress::ToolCallStarted { iteration, .. } => iteration,
            AgentProgress::TextDelta { iteration, .. } => iteration,
            other => panic!("unexpected event: {other:?}"),
        })
        .collect();
    assert_eq!(iterations, vec![1, 2, 2]);
}

#[tokio::test]
async fn started_resets_the_round_counter_for_a_reused_sink() {
    let (sink, mut rx) = sink(16);
    sink.emit(tool_call("a")).await;
    sink.emit(started()).await;
    sink.emit(tool_call("b")).await;

    let mut iterations = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let AgentProgress::ToolCallStarted { iteration, .. } = ev {
            iterations.push(iteration);
        }
    }
    assert_eq!(iterations, vec![1, 1], "a new turn restarts at round 1");
}

#[tokio::test]
async fn finished_maps_to_turn_completed_carrying_the_round_count() {
    let (sink, mut rx) = sink(16);
    sink.emit(tool_call("a")).await;
    let _ = rx.try_recv();

    sink.emit(ProgressEvent::Finished {
        run: run(),
        usage: Some(Usage {
            input_tokens: 12,
            output_tokens: 3,
            total_tokens: 15,
            ..Usage::default()
        }),
    })
    .await;

    match rx.try_recv().expect("event forwarded") {
        AgentProgress::TurnCompleted { iterations } => assert_eq!(iterations, 2),
        other => panic!("unexpected event: {other:?}"),
    }
    // Usage is deliberately NOT projected onto TurnCostUpdated — see the
    // module docs. A fabricated cost update would under-report in the UI.
    assert!(rx.try_recv().is_err(), "no cost event is synthesised");
}

#[tokio::test]
async fn finished_without_usage_still_completes_the_turn() {
    let (sink, mut rx) = sink(8);
    sink.emit(ProgressEvent::Finished {
        run: run(),
        usage: None,
    })
    .await;
    assert!(matches!(
        rx.try_recv().expect("event forwarded"),
        AgentProgress::TurnCompleted { iterations: 1 }
    ));
}

#[tokio::test]
async fn error_forwards_nothing() {
    let (sink, mut rx) = sink(8);
    sink.emit(ProgressEvent::Error {
        run: run(),
        message: "provider unavailable".to_string(),
    })
    .await;
    // Reporting a failure as a completion would corrupt both the run ledger
    // and the chat timeline; the turn's own Err is the authoritative path.
    assert!(rx.try_recv().is_err());
    assert_eq!(sink.dropped(), 0, "not forwarding is not dropping");
}

#[tokio::test]
async fn a_permanently_full_channel_drops_instead_of_stalling_the_turn() {
    let (sink, mut rx) = sink(1);
    sink.emit(started()).await;
    // Nothing ever drains, so the grace window expires and the events are
    // dropped. The turn must still finish rather than park forever.
    sink.emit(tool_call("a")).await;
    sink.emit(tool_call("b")).await;

    assert!(matches!(
        rx.try_recv().expect("first event fits"),
        AgentProgress::TurnStarted
    ));
    assert_eq!(sink.dropped(), 2);
}

#[tokio::test]
async fn a_lifecycle_event_survives_transient_backpressure_that_drops_a_delta() {
    // The regression: a burst of deltas fills the channel, and a
    // `ToolCallStarted` lost in that window leaves the tool row stuck in
    // `running` forever. A delta lost in the same window costs one UI tick.
    let (sink, mut rx) = sink(1);
    sink.emit(started()).await;

    // A delta finds the channel full and is dropped immediately.
    sink.emit(ProgressEvent::Token {
        run: run(),
        text: "hi".to_string(),
    })
    .await;
    assert_eq!(sink.dropped(), 1, "deltas never wait");

    // Drive the blocked send and the consumer concurrently on one task, so
    // the ordering comes from poll order rather than from wall-clock sleeps
    // racing the grace window — the send registers as a waiter, then the
    // first `recv` frees a slot and wakes it. No margin to lose under load.
    let consumer = async {
        let first = rx.recv().await;
        let second = rx.recv().await;
        (first, second)
    };
    let ((), (first, second)) = tokio::join!(sink.emit(tool_call("a")), consumer);
    assert!(matches!(first, Some(AgentProgress::TurnStarted)));
    assert!(
        matches!(second, Some(AgentProgress::ToolCallStarted { .. })),
        "the lifecycle event must ride out the transient window, got {second:?}"
    );
    assert_eq!(sink.dropped(), 1, "only the delta was dropped");
}

#[tokio::test]
async fn a_closed_channel_is_survivable() {
    let (sink, rx) = sink(8);
    drop(rx);
    // A UI that hung up must not be able to fail the turn.
    sink.emit(started()).await;
    sink.emit(ProgressEvent::Finished {
        run: run(),
        usage: None,
    })
    .await;
    assert_eq!(sink.dropped(), 2);
}

#[tokio::test]
async fn works_through_an_arc_trait_object() {
    let (tx, mut rx) = mpsc::channel(8);
    let dynamic: Arc<dyn ProgressSink> = Arc::new(OpenHumanProgressSink::new(tx));
    dynamic.emit(started()).await;
    assert!(matches!(
        rx.try_recv().expect("event forwarded"),
        AgentProgress::TurnStarted
    ));
}
