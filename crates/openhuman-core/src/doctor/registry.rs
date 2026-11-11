//! Built-in checks and the generic host check-source registry.

use std::collections::HashSet;
use std::sync::Arc;

use crate::core::slots::{Extensions, HookSlot};

use super::contract::{DoctorCheck, DoctorCtx};

/// A host-owned group of checks. The core stores the source without knowing
/// which host or service supplies it.
pub trait DoctorCheckSource: Send + Sync {
    /// Stable source id, used for replacement and diagnostics.
    fn id(&self) -> &str;
    /// Checks contributed by this source.
    fn checks(&self) -> Vec<Arc<dyn DoctorCheck>>;
    /// Checks contributed for the selected doctor configuration.
    ///
    /// Existing host sources remain context-free by default. Sources whose
    /// identifiers or check set depends on the effective configuration can
    /// override this method and must use `ctx`, not ambient process state.
    fn checks_for(&self, _ctx: &DoctorCtx) -> Vec<Arc<dyn DoctorCheck>> {
        self.checks()
    }
}

static CHECK_SOURCES: HookSlot<Arc<dyn DoctorCheckSource>> =
    HookSlot::new("doctor::registry::sources");

/// Registers a check source for the whole process.
pub fn register_check_source(source: impl DoctorCheckSource + 'static) {
    let source: Arc<dyn DoctorCheckSource> = Arc::new(source);
    let id = source.id().to_owned();
    CHECK_SOURCES.register(id, source);
}

/// Registers a check source for the context built from `extensions`.
pub fn register_check_source_in(extensions: &Extensions, source: impl DoctorCheckSource + 'static) {
    let source: Arc<dyn DoctorCheckSource> = Arc::new(source);
    let id = source.id().to_owned();
    CHECK_SOURCES.register_in(extensions, id, source);
}

/// Removes a process-wide source, primarily for isolated tests and hosts that
/// explicitly shut down a dynamically loaded check provider.
pub fn unregister_check_source(id: &str) -> bool {
    CHECK_SOURCES.unregister(id)
}

/// The checks available in the active context, with duplicate and malformed
/// ids rejected before a report starts.
pub fn registered_checks() -> Result<Vec<Arc<dyn DoctorCheck>>, String> {
    let mut checks = Vec::new();
    checks.extend(super::profile::built_in_checks());
    checks.extend(super::checks::builtins());
    for (_, source) in CHECK_SOURCES.entries() {
        checks.extend(source.checks());
    }

    validate_checks(checks)
}

/// The checks available for the selected doctor context, with duplicate and
/// malformed ids rejected before a report starts.
pub fn registered_checks_for(ctx: &DoctorCtx) -> Result<Vec<Arc<dyn DoctorCheck>>, String> {
    let mut checks = Vec::new();
    checks.extend(super::profile::built_in_checks());
    checks.extend(super::checks::builtins());
    checks.extend(super::checks::for_context(ctx));
    for (_, source) in CHECK_SOURCES.entries() {
        checks.extend(source.checks_for(ctx));
    }

    validate_checks(checks)
}

fn validate_checks(checks: Vec<Arc<dyn DoctorCheck>>) -> Result<Vec<Arc<dyn DoctorCheck>>, String> {
    let mut ids = HashSet::with_capacity(checks.len());
    for check in &checks {
        let id = check.id();
        if id == "doctor/selection" {
            return Err("doctor/selection is reserved by the runner".into());
        }
        if !valid_check_id(id) {
            return Err(format!(
                "doctor check id must use lowercase area/name form: {id}"
            ));
        }
        if !ids.insert(id.to_owned()) {
            return Err(format!("duplicate doctor check id: {id}"));
        }
    }
    Ok(checks)
}

fn valid_check_id(id: &str) -> bool {
    let mut segments = 0;
    for segment in id.split('/') {
        if segment.is_empty()
            || !segment.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
        {
            return false;
        }
        segments += 1;
    }
    segments >= 2
}
