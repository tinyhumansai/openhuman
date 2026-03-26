use super::traits::{Observer, ObserverEvent, ObserverMetric};
use std::any::Any;

/// Zero-overhead observer — all methods compile to nothing
pub struct NoopObserver;

impl Observer for NoopObserver {
    #[inline(always)]
    fn record_event(&self, _event: &ObserverEvent) {}

    #[inline(always)]
    fn record_metric(&self, _metric: &ObserverMetric) {}

    fn name(&self) -> &str {
        "noop"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn noop_name() {
        assert_eq!(NoopObserver.name(), "noop");
    }

    #[test]
    fn noop_record_event_does_not_panic() {
        let obs = NoopObserver;
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "test".into(),
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            provider: "test".into(),
            model: "test".into(),
            duration: Duration::from_millis(100),
            tokens_used: Some(42),
            cost_usd: Some(0.001),
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            provider: "test".into(),
            model: "test".into(),
            duration: Duration::ZERO,
            tokens_used: None,
            cost_usd: None,
        });
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_secs(1),
            success: true,
        });
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "cli".into(),
            direction: "inbound".into(),
        });
        obs.record_event(&ObserverEvent::Error {
            component: "test".into(),
            message: "boom".into(),
        });
    }

    #[test]
    fn noop_record_metric_does_not_panic() {
        let obs = NoopObserver;
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_millis(50)));
        obs.record_metric(&ObserverMetric::TokensUsed(1000));
        obs.record_metric(&ObserverMetric::ActiveSessions(5));
        obs.record_metric(&ObserverMetric::QueueDepth(0));
    }

    #[test]
    fn noop_flush_does_not_panic() {
        NoopObserver.flush();
    }
}
