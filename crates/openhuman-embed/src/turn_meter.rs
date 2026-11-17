//! A turn's usage callback survives dispatch errors, cancellation and dropping.

use openhuman_core::agent::tinyagents::host::LastTurnUsage;

pub(crate) type UsageSink = std::sync::Mutex<Option<LastTurnUsage>>;

pub(crate) struct TurnMeter {
    pub(crate) usage: UsageSink,
    callback: Option<Box<dyn FnOnce(Option<LastTurnUsage>) + Send>>,
}

impl TurnMeter {
    pub(crate) fn new(callback: Option<Box<dyn FnOnce(Option<LastTurnUsage>) + Send>>) -> Self {
        Self {
            usage: std::sync::Mutex::new(None),
            callback,
        }
    }
}

impl Drop for TurnMeter {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.take() {
            callback(
                self.usage
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            );
        }
    }
}
