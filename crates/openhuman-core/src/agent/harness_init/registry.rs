//! The set of one-time initialization steps run eagerly at core startup.

use crate::config::Config;
use std::future::Future;
use std::pin::Pin;

/// Future returned by a step's probe/run hooks.
pub type StepFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A single startup provisioning step.
pub struct HarnessInitStep {
    pub id: &'static str,
    pub label: &'static str,
    pub required: bool,
    pub provisioning: bool,
    pub is_done: for<'a> fn(&'a Config) -> StepFuture<'a, bool>,
    pub run: for<'a> fn(&'a Config) -> StepFuture<'a, Result<(), String>>,
}

/// The ordered list of initialization steps. Host runtimes are inherited from PATH.
pub fn all_steps() -> Vec<HarnessInitStep> {
    Vec::new()
}
