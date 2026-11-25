//! Agent tools for the cron scheduler.
//!
//! The six per-operation tools are what the model sees, as the `scheduling`
//! pack. [`CronTool`] dispatches the same six on one `action` field and stays
//! registered, deferred, for callers that already name `cron`; see
//! [`collapsed`].

mod add;
mod collapsed;
mod list;
mod remove;
mod run;
mod runs;
mod schedule;
mod update;

pub use add::CronAddTool;
pub use collapsed::{CronTool, CRON_TOOL_NAME};
pub use list::CronListTool;
pub use remove::CronRemoveTool;
pub use run::CronRunTool;
pub use runs::CronRunsTool;
pub use schedule::ScheduleTool;
pub use update::CronUpdateTool;

/// The per-operation scheduler tools: the `scheduling` pack's members and
/// what the model is shown, in the order the pack lists them.
pub const CRON_OPERATION_TOOLS: &[&str] = &[
    "cron_add",
    "cron_list",
    "cron_update",
    "cron_remove",
    "cron_run",
    "cron_runs",
];

/// Widens `names`, a named tool scope, so that naming the collapsed `cron`
/// also names [`CRON_OPERATION_TOOLS`]. `cron` is deferred, so a scope written
/// before the split would otherwise list no scheduler tool up front.
pub fn widen_named_scope(names: &mut Vec<String>) {
    if !names.iter().any(|name| name == CRON_TOOL_NAME) {
        return;
    }
    for tool in CRON_OPERATION_TOOLS {
        if !names.iter().any(|name| name == tool) {
            names.push((*tool).to_string());
        }
    }
    log::debug!("[cron][tools] named scope lists `cron`; widened to its per-operation tools");
}

/// Register the per-operation scheduler tools and the deferred collapsed `cron`.
pub(crate) fn tools(
    config: std::sync::Arc<crate::config::Config>,
    security: &std::sync::Arc<crate::security::SecurityPolicy>,
) -> Vec<Box<dyn tinytools::Tool>> {
    vec![
        Box::new(CronTool::new(config.clone(), security.clone())),
        Box::new(CronAddTool::new(config.clone(), security.clone())),
        Box::new(CronListTool::new(config.clone())),
        Box::new(CronRemoveTool::new(config.clone())),
        Box::new(CronUpdateTool::new(config.clone(), security.clone())),
        Box::new(CronRunTool::new(config.clone())),
        Box::new(CronRunsTool::new(config)),
    ]
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
