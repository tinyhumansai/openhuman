//! Agent tools for the cron scheduler.
//!
//! The six per-operation tools are the implementation; [`CronTool`] is what the
//! model sees. See [`collapsed`] for why they are separate and why the six stay
//! registered.

mod add;
mod collapsed;
mod list;
mod remove;
mod run;
mod runs;
mod update;

pub use add::CronAddTool;
pub use collapsed::{CronTool, CRON_TOOL_NAME};
pub use list::CronListTool;
pub use remove::CronRemoveTool;
pub use run::CronRunTool;
pub use runs::CronRunsTool;
pub use update::CronUpdateTool;

/// Register the advertised scheduler and its replay-compatible operation aliases.
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
