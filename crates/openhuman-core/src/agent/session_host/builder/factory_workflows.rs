//! The skill and flow catalogue exposed to a newly constructed session.

use crate::config::Config;
use tinyskills::Skill as Workflow;

pub(super) fn session_workflows(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    host_only: bool,
) -> Vec<Workflow> {
    if host_only {
        return Vec::new();
    }
    #[cfg(all(feature = "flows", feature = "skills"))]
    crate::skills::schedule_flow_authoring_prewarm(ctx, config, None);
    let catalogue = crate::skills::load_workflow_metadata_for(ctx, config);
    #[cfg(feature = "flows")]
    let catalogue = {
        let mut catalogue = catalogue;
        catalogue.extend(crate::flows::catalogue::flow_entries(config));
        catalogue
    };
    catalogue
}
