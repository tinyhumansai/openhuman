use super::traits::{Observer, ObserverEvent, ObserverMetric};
use std::any::Any;

/// Human-readable progress observer for interactive sessions.
///
/// This observer prints compact `>` / `<` progress lines without exposing
/// prompt contents. It is intended to be opt-in (e.g. `--verbose`).
pub struct VerboseObserver;

impl VerboseObserver {
    pub fn new() -> Self {
        Self
    }
}

impl Observer for VerboseObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::LlmRequest {
                provider,
                model,
                messages_count,
            } => {
                eprintln!("> Thinking");
                eprintln!(
                    "> Send (provider={}, model={}, messages={})",
                    provider, model, messages_count
                );
            }
            ObserverEvent::LlmResponse {
                duration, success, ..
            } => {
                let ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
                eprintln!("< Receive (success={success}, duration_ms={ms})");
            }
            ObserverEvent::ToolCallStart { tool } => {
                eprintln!("> Tool {tool}");
            }
            ObserverEvent::ToolCall {
                tool,
                duration,
                success,
            } => {
                let ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
                eprintln!("< Tool {tool} (success={success}, duration_ms={ms})");
            }
            ObserverEvent::TurnComplete => {
                eprintln!("< Complete");
            }
            _ => {}
        }
    }

    #[inline(always)]
    fn record_metric(&self, _metric: &ObserverMetric) {}

    fn name(&self) -> &str {
        "verbose"
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
    fn verbose_name() {
        assert_eq!(VerboseObserver::new().name(), "verbose");
    }

    #[test]
    fn verbose_events_do_not_panic() {
        let obs = VerboseObserver::new();
        obs.record_event(&ObserverEvent::LlmRequest {
            provider: "openrouter".into(),
            model: "claude".into(),
            messages_count: 3,
        });
        obs.record_event(&ObserverEvent::LlmResponse {
            provider: "openrouter".into(),
            model: "claude".into(),
            duration: Duration::from_millis(12),
            success: true,
            error_message: None,
        });
        obs.record_event(&ObserverEvent::ToolCallStart {
            tool: "shell".into(),
        });
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_millis(2),
            success: true,
        });
        obs.record_event(&ObserverEvent::TurnComplete);
    }
}
