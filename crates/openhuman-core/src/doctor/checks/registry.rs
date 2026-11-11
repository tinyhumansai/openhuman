//! Composes native checks from the selected doctor configuration.

use std::sync::Arc;

use super::super::contract::{DoctorCheck, DoctorCtx};

/// Constructs checks from the configuration selected for this report.
pub(in crate::doctor) fn for_context(ctx: &DoctorCtx) -> Vec<Arc<dyn DoctorCheck>> {
    let mut checks = super::modules::for_context(ctx);
    checks.extend(super::inference_live::for_context(ctx));
    checks.extend(super::mcp::for_context(ctx));
    checks.extend(super::skills::for_context(ctx));
    checks.extend(super::sandbox::for_context(ctx));
    checks.extend(super::channels::for_context(ctx));
    checks.extend(super::integrations::for_context());
    checks.extend(super::voice::for_context());
    checks.extend(super::search::for_context(ctx));
    checks.extend(super::wallet::for_context());
    checks
}
