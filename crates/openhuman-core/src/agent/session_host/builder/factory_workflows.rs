//! The skill and flow catalogue exposed to a newly constructed session.

use crate::config::Config;
use tinyskills::Skill as Workflow;

pub(super) fn session_workflows(config: &Config, host_only: bool) -> Vec<Workflow> {
    if host_only {
        return Vec::new();
    }
    let catalogue = crate::skills::load_workflow_metadata(&config.workspace_dir);
    #[cfg(feature = "flows")]
    let catalogue = {
        let mut catalogue = catalogue;
        catalogue.extend(crate::flows::catalogue::flow_entries(config));
        catalogue
    };
    catalogue
}
