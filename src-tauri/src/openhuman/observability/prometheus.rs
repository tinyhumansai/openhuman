use super::traits::{Observer, ObserverEvent, ObserverMetric};
use prometheus::{
    Encoder, GaugeVec, Histogram, HistogramOpts, HistogramVec, IntCounterVec, Registry, TextEncoder,
};

/// Prometheus-backed observer — exposes metrics for scraping via `/metrics`.
pub struct PrometheusObserver {
    registry: Registry,

    // Counters
    agent_starts: IntCounterVec,
    tool_calls: IntCounterVec,
    channel_messages: IntCounterVec,
    heartbeat_ticks: prometheus::IntCounter,
    errors: IntCounterVec,

    // Histograms
    agent_duration: HistogramVec,
    tool_duration: HistogramVec,
    request_latency: Histogram,

    // Gauges
    tokens_used: prometheus::IntGauge,
    active_sessions: GaugeVec,
    queue_depth: GaugeVec,
}

impl PrometheusObserver {
    pub fn new() -> Self {
        let registry = Registry::new();

        let agent_starts = IntCounterVec::new(
            prometheus::Opts::new("openhuman_agent_starts_total", "Total agent invocations"),
            &["provider", "model"],
        )
        .expect("valid metric");

        let tool_calls = IntCounterVec::new(
            prometheus::Opts::new("openhuman_tool_calls_total", "Total tool calls"),
            &["tool", "success"],
        )
        .expect("valid metric");

        let channel_messages = IntCounterVec::new(
            prometheus::Opts::new("openhuman_channel_messages_total", "Total channel messages"),
            &["channel", "direction"],
        )
        .expect("valid metric");

        let heartbeat_ticks =
            prometheus::IntCounter::new("openhuman_heartbeat_ticks_total", "Total heartbeat ticks")
                .expect("valid metric");

        let errors = IntCounterVec::new(
            prometheus::Opts::new("openhuman_errors_total", "Total errors by component"),
            &["component"],
        )
        .expect("valid metric");

        let agent_duration = HistogramVec::new(
            HistogramOpts::new(
                "openhuman_agent_duration_seconds",
                "Agent invocation duration in seconds",
            )
            .buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0]),
            &["provider", "model"],
        )
        .expect("valid metric");

        let tool_duration = HistogramVec::new(
            HistogramOpts::new(
                "openhuman_tool_duration_seconds",
                "Tool execution duration in seconds",
            )
            .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0]),
            &["tool"],
        )
        .expect("valid metric");

        let request_latency = Histogram::with_opts(
            HistogramOpts::new(
                "openhuman_request_latency_seconds",
                "Request latency in seconds",
            )
            .buckets(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        )
        .expect("valid metric");

        let tokens_used = prometheus::IntGauge::new(
            "openhuman_tokens_used_last",
            "Tokens used in the last request",
        )
        .expect("valid metric");

        let active_sessions = GaugeVec::new(
            prometheus::Opts::new("openhuman_active_sessions", "Number of active sessions"),
            &[],
        )
        .expect("valid metric");

        let queue_depth = GaugeVec::new(
            prometheus::Opts::new("openhuman_queue_depth", "Message queue depth"),
            &[],
        )
        .expect("valid metric");

        // Register all metrics
        registry.register(Box::new(agent_starts.clone())).ok();
        registry.register(Box::new(tool_calls.clone())).ok();
        registry.register(Box::new(channel_messages.clone())).ok();
        registry.register(Box::new(heartbeat_ticks.clone())).ok();
        registry.register(Box::new(errors.clone())).ok();
        registry.register(Box::new(agent_duration.clone())).ok();
        registry.register(Box::new(tool_duration.clone())).ok();
        registry.register(Box::new(request_latency.clone())).ok();
        registry.register(Box::new(tokens_used.clone())).ok();
        registry.register(Box::new(active_sessions.clone())).ok();
        registry.register(Box::new(queue_depth.clone())).ok();

        Self {
            registry,
            agent_starts,
            tool_calls,
            channel_messages,
            heartbeat_ticks,
            errors,
            agent_duration,
            tool_duration,
            request_latency,
            tokens_used,
            active_sessions,
            queue_depth,
        }
    }

    /// Encode all registered metrics into Prometheus text exposition format.
    pub fn encode(&self) -> String {
        let encoder = TextEncoder::new();
        let families = self.registry.gather();
        let mut buf = Vec::new();
        encoder.encode(&families, &mut buf).unwrap_or_default();
        String::from_utf8(buf).unwrap_or_default()
    }
}

impl Observer for PrometheusObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { provider, model } => {
                self.agent_starts
                    .with_label_values(&[provider, model])
                    .inc();
            }
            ObserverEvent::AgentEnd {
                provider,
                model,
                duration,
                tokens_used,
                cost_usd: _,
            } => {
                // Agent duration is recorded via the histogram with provider/model labels
                self.agent_duration
                    .with_label_values(&[provider, model])
                    .observe(duration.as_secs_f64());
                if let Some(t) = tokens_used {
                    self.tokens_used.set(i64::try_from(*t).unwrap_or(i64::MAX));
                }
            }
            ObserverEvent::ToolCallStart { tool: _ }
            | ObserverEvent::TurnComplete
            | ObserverEvent::LlmRequest { .. }
            | ObserverEvent::LlmResponse { .. } => {}
            ObserverEvent::ToolCall {
                tool,
                duration,
                success,
            } => {
                let success_str = if *success { "true" } else { "false" };
                self.tool_calls
                    .with_label_values(&[tool.as_str(), success_str])
                    .inc();
                self.tool_duration
                    .with_label_values(&[tool.as_str()])
                    .observe(duration.as_secs_f64());
            }
            ObserverEvent::ChannelMessage { channel, direction } => {
                self.channel_messages
                    .with_label_values(&[channel, direction])
                    .inc();
            }
            ObserverEvent::HeartbeatTick => {
                self.heartbeat_ticks.inc();
            }
            ObserverEvent::Error {
                component,
                message: _,
            } => {
                self.errors.with_label_values(&[component]).inc();
            }
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        match metric {
            ObserverMetric::RequestLatency(d) => {
                self.request_latency.observe(d.as_secs_f64());
            }
            ObserverMetric::TokensUsed(t) => {
                self.tokens_used.set(i64::try_from(*t).unwrap_or(i64::MAX));
            }
            ObserverMetric::ActiveSessions(s) => {
                self.active_sessions
                    .with_label_values(&[] as &[&str])
                    .set(*s as f64);
            }
            ObserverMetric::QueueDepth(d) => {
                self.queue_depth
                    .with_label_values(&[] as &[&str])
                    .set(*d as f64);
            }
        }
    }

    fn name(&self) -> &str {
        "prometheus"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn prometheus_observer_name() {
        assert_eq!(PrometheusObserver::new().name(), "prometheus");
    }

    #[test]
    fn records_all_events_without_panic() {
        let obs = PrometheusObserver::new();
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "openrouter".into(),
            model: "claude-sonnet".into(),
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            provider: "openrouter".into(),
            model: "claude-sonnet".into(),
            duration: Duration::from_millis(500),
            tokens_used: Some(100),
            cost_usd: None,
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            provider: "openrouter".into(),
            model: "claude-sonnet".into(),
            duration: Duration::ZERO,
            tokens_used: None,
            cost_usd: None,
        });
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_millis(10),
            success: true,
        });
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "file_read".into(),
            duration: Duration::from_millis(5),
            success: false,
        });
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "telegram".into(),
            direction: "inbound".into(),
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::Error {
            component: "provider".into(),
            message: "timeout".into(),
        });
    }

    #[test]
    fn records_all_metrics_without_panic() {
        let obs = PrometheusObserver::new();
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_secs(2)));
        obs.record_metric(&ObserverMetric::TokensUsed(500));
        obs.record_metric(&ObserverMetric::TokensUsed(0));
        obs.record_metric(&ObserverMetric::ActiveSessions(3));
        obs.record_metric(&ObserverMetric::QueueDepth(42));
    }

    #[test]
    fn encode_produces_prometheus_text_format() {
        let obs = PrometheusObserver::new();
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "openrouter".into(),
            model: "claude-sonnet".into(),
        });
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_millis(100),
            success: true,
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_millis(250)));

        let output = obs.encode();
        assert!(output.contains("openhuman_agent_starts_total"));
        assert!(output.contains("openhuman_tool_calls_total"));
        assert!(output.contains("openhuman_heartbeat_ticks_total"));
        assert!(output.contains("openhuman_request_latency_seconds"));
    }

    #[test]
    fn counters_increment_correctly() {
        let obs = PrometheusObserver::new();

        for _ in 0..3 {
            obs.record_event(&ObserverEvent::HeartbeatTick);
        }

        let output = obs.encode();
        assert!(output.contains("openhuman_heartbeat_ticks_total 3"));
    }

    #[test]
    fn tool_calls_track_success_and_failure_separately() {
        let obs = PrometheusObserver::new();

        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_millis(10),
            success: true,
        });
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_millis(10),
            success: true,
        });
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_millis(10),
            success: false,
        });

        let output = obs.encode();
        assert!(output.contains(r#"openhuman_tool_calls_total{success="true",tool="shell"} 2"#));
        assert!(output.contains(r#"openhuman_tool_calls_total{success="false",tool="shell"} 1"#));
    }

    #[test]
    fn errors_track_by_component() {
        let obs = PrometheusObserver::new();
        obs.record_event(&ObserverEvent::Error {
            component: "provider".into(),
            message: "timeout".into(),
        });
        obs.record_event(&ObserverEvent::Error {
            component: "provider".into(),
            message: "rate limit".into(),
        });
        obs.record_event(&ObserverEvent::Error {
            component: "channels".into(),
            message: "disconnected".into(),
        });

        let output = obs.encode();
        assert!(output.contains(r#"openhuman_errors_total{component="provider"} 2"#));
        assert!(output.contains(r#"openhuman_errors_total{component="channels"} 1"#));
    }

    #[test]
    fn gauge_reflects_latest_value() {
        let obs = PrometheusObserver::new();
        obs.record_metric(&ObserverMetric::TokensUsed(100));
        obs.record_metric(&ObserverMetric::TokensUsed(200));

        let output = obs.encode();
        assert!(output.contains("openhuman_tokens_used_last 200"));
    }
}
