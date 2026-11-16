//! Shared fixtures for the update tests.

use std::sync::Mutex;

use super::types::{UpdateReporter, UpdateSource};

/// The source the core ships with.
pub(crate) const SOURCE: UpdateSource = UpdateSource {
    owner: "tinyhumansai",
    repo: "openhuman",
    asset_prefix: "openhuman-core",
};

/// A running version every mocked `v99.0.0` release is newer than.
pub(crate) const RUNNING_VERSION: &str = "1.0.0";

/// A reporter that records what it was asked to report.
#[derive(Default)]
pub(crate) struct RecordingReporter {
    reports: Mutex<Vec<(String, String)>>,
}

impl RecordingReporter {
    /// `(operation, message)` for every report so far. Only the missing-asset
    /// test reads it, and that test is compiled off Linux (Linux release
    /// assets are always published).
    #[cfg(not(target_os = "linux"))]
    pub(crate) fn reports(&self) -> Vec<(String, String)> {
        self.reports.lock().expect("reports lock").clone()
    }
}

impl UpdateReporter for RecordingReporter {
    fn report(&self, message: &str, operation: &str, _tags: &[(&str, &str)]) {
        self.reports
            .lock()
            .expect("reports lock")
            .push((operation.to_string(), message.to_string()));
    }
}
