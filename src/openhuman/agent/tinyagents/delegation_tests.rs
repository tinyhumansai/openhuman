use super::*;

/// The graph mechanics, the revision budget, checkpoint/resume
/// classification and the on-disk state shape are all tested upstream in
/// `tinyagents_graph::delegation`. What is testable *here* is the only
/// behaviour this file adds: that a run gets OpenHuman's tracing sink
/// attached, and that an explicit one is never overridden.
#[test]
fn tracing_sink_is_attached_when_the_caller_supplies_none() {
    let config = with_tracing_sink(DelegationConfig::default());
    assert!(
        config.event_sink.is_some(),
        "every delegation run must be journalled onto openhuman tracing"
    );
}

#[test]
fn an_explicit_sink_is_not_overridden() {
    let mine: Arc<dyn tinyagents_graph::stream::GraphEventSink> =
        Arc::new(tinyagents_graph::stream::CollectingSink::default());
    let config = with_tracing_sink(DelegationConfig {
        event_sink: Some(mine.clone()),
        ..DelegationConfig::default()
    });
    let attached = config.event_sink.expect("kept");
    assert!(
        Arc::ptr_eq(&mine, &attached),
        "a caller-supplied sink is kept, not replaced"
    );
}
