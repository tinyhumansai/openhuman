//! Trusted identity mapping shared by TinySecurity host adapters.

use std::path::Path;

use tinysecurity_bus::SubjectScope;

use crate::core::runtime::tenant::Tenant;

/// Map the authenticated task tenant to the subject handle registered with TinySecurity.
pub(crate) fn subject_scope(
    tenant: &Tenant,
    workspace_dir: &Path,
    saas: bool,
) -> Result<SubjectScope, String> {
    let user = match tenant
        .profile
        .as_deref()
        .filter(|profile| !profile.trim().is_empty())
    {
        Some(profile) => profile.to_owned(),
        None if saas => {
            return Err("TinySecurity approval scope requires an authenticated profile".into());
        }
        None => "local".to_owned(),
    };
    Ok(SubjectScope {
        user,
        workspace: workspace_dir.to_string_lossy().into_owned(),
        agent: tenant
            .agent
            .as_deref()
            .filter(|agent| !agent.trim().is_empty())
            .unwrap_or("orchestrator")
            .to_owned(),
    })
}
