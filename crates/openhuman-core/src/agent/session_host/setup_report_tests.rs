use std::sync::{Arc, Mutex};

use tinyagents_harness::observability::turn_observer::{
    with_observer, Observer, ObserverScope, SetupOutcome, SetupPhase, TraceContent, TurnObservation,
};

use super::*;

#[derive(Default)]
struct Recorder(Mutex<Vec<TurnObservation>>);

impl Observer for Recorder {
    fn on_event(&self, event: &TurnObservation) {
        self.0.lock().unwrap().push(event.clone());
    }
}

async fn observed<T>(work: impl Future<Output = T>) -> (T, Vec<TurnObservation>) {
    let recorder = Arc::new(Recorder::default());
    let scope = ObserverScope::new(recorder.clone(), TraceContent::MetadataOnly);
    let out = with_observer(scope, work).await;
    let events = recorder.0.lock().unwrap().clone();
    (out, events)
}

#[tokio::test]
async fn a_scoped_observer_hears_each_phase_with_its_outcome_and_elapsed_time() {
    let (_, events) = observed(async {
        reported(
            SetupPhase::PromptRefresh,
            async { tokio::time::sleep(std::time::Duration::from_millis(20)).await },
            |_| SetupOutcome::Completed,
        )
        .await;
        reported(
            SetupPhase::MemoryBlock,
            async { Err::<(), &str>("down") },
            result_outcome,
        )
        .await
        .unwrap_err();
        reported(SetupPhase::MemoryBlock, async {}, |_| SetupOutcome::Skipped).await;
    })
    .await;

    let shape: Vec<_> = events
        .iter()
        .map(|event| match event {
            TurnObservation::Setup {
                phase,
                failed,
                skipped,
                duration_ms,
                ..
            } => (phase.as_str(), *failed, *skipped, *duration_ms >= 20),
            other => panic!("only setup phases are reported here: {other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            ("prompt_refresh", false, false, true),
            ("memory_block", true, false, false),
            ("memory_block", false, true, false),
        ]
    );
}

#[tokio::test]
async fn reporting_without_a_scoped_observer_returns_the_result_untouched() {
    let value = reported(SetupPhase::AgentOpen, async { 7 }, |_| {
        SetupOutcome::Completed
    })
    .await;
    assert_eq!(value, 7);
}
