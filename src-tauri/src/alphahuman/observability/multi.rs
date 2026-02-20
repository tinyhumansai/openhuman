use super::traits::{Observer, ObserverEvent, ObserverMetric};
use std::any::Any;

/// Combine multiple observers — fan-out events to all backends
pub struct MultiObserver {
    observers: Vec<Box<dyn Observer>>,
}

impl MultiObserver {
    pub fn new(observers: Vec<Box<dyn Observer>>) -> Self {
        Self { observers }
    }
}

impl Observer for MultiObserver {
    fn record_event(&self, event: &ObserverEvent) {
        for obs in &self.observers {
            obs.record_event(event);
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        for obs in &self.observers {
            obs.record_metric(metric);
        }
    }

    fn flush(&self) {
        for obs in &self.observers {
            obs.flush();
        }
    }

    fn name(&self) -> &str {
        "multi"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Test observer that counts calls
    struct CountingObserver {
        event_count: Arc<AtomicUsize>,
        metric_count: Arc<AtomicUsize>,
        flush_count: Arc<AtomicUsize>,
    }

    impl CountingObserver {
        fn new(
            event_count: Arc<AtomicUsize>,
            metric_count: Arc<AtomicUsize>,
            flush_count: Arc<AtomicUsize>,
        ) -> Self {
            Self {
                event_count,
                metric_count,
                flush_count,
            }
        }
    }

    impl Observer for CountingObserver {
        fn record_event(&self, _event: &ObserverEvent) {
            self.event_count.fetch_add(1, Ordering::SeqCst);
        }
        fn record_metric(&self, _metric: &ObserverMetric) {
            self.metric_count.fetch_add(1, Ordering::SeqCst);
        }
        fn flush(&self) {
            self.flush_count.fetch_add(1, Ordering::SeqCst);
        }
        fn name(&self) -> &str {
            "counting"
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn multi_name() {
        let m = MultiObserver::new(vec![]);
        assert_eq!(m.name(), "multi");
    }

    #[test]
    fn multi_empty_no_panic() {
        let m = MultiObserver::new(vec![]);
        m.record_event(&ObserverEvent::HeartbeatTick);
        m.record_metric(&ObserverMetric::TokensUsed(10));
        m.flush();
    }

    #[test]
    fn multi_fans_out_events() {
        let ec1 = Arc::new(AtomicUsize::new(0));
        let mc1 = Arc::new(AtomicUsize::new(0));
        let fc1 = Arc::new(AtomicUsize::new(0));
        let ec2 = Arc::new(AtomicUsize::new(0));
        let mc2 = Arc::new(AtomicUsize::new(0));
        let fc2 = Arc::new(AtomicUsize::new(0));

        let m = MultiObserver::new(vec![
            Box::new(CountingObserver::new(ec1.clone(), mc1.clone(), fc1.clone())),
            Box::new(CountingObserver::new(ec2.clone(), mc2.clone(), fc2.clone())),
        ]);

        m.record_event(&ObserverEvent::HeartbeatTick);
        m.record_event(&ObserverEvent::HeartbeatTick);
        m.record_event(&ObserverEvent::HeartbeatTick);

        assert_eq!(ec1.load(Ordering::SeqCst), 3);
        assert_eq!(ec2.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn multi_fans_out_metrics() {
        let ec1 = Arc::new(AtomicUsize::new(0));
        let mc1 = Arc::new(AtomicUsize::new(0));
        let fc1 = Arc::new(AtomicUsize::new(0));
        let ec2 = Arc::new(AtomicUsize::new(0));
        let mc2 = Arc::new(AtomicUsize::new(0));
        let fc2 = Arc::new(AtomicUsize::new(0));

        let m = MultiObserver::new(vec![
            Box::new(CountingObserver::new(ec1.clone(), mc1.clone(), fc1.clone())),
            Box::new(CountingObserver::new(ec2.clone(), mc2.clone(), fc2.clone())),
        ]);

        m.record_metric(&ObserverMetric::TokensUsed(100));
        m.record_metric(&ObserverMetric::RequestLatency(Duration::from_millis(5)));

        assert_eq!(mc1.load(Ordering::SeqCst), 2);
        assert_eq!(mc2.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn multi_fans_out_flush() {
        let ec = Arc::new(AtomicUsize::new(0));
        let mc = Arc::new(AtomicUsize::new(0));
        let fc1 = Arc::new(AtomicUsize::new(0));
        let fc2 = Arc::new(AtomicUsize::new(0));

        let m = MultiObserver::new(vec![
            Box::new(CountingObserver::new(ec.clone(), mc.clone(), fc1.clone())),
            Box::new(CountingObserver::new(ec.clone(), mc.clone(), fc2.clone())),
        ]);

        m.flush();
        assert_eq!(fc1.load(Ordering::SeqCst), 1);
        assert_eq!(fc2.load(Ordering::SeqCst), 1);
    }
}
