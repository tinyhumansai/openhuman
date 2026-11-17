//! Static module lifecycle and provenance checks.

use std::sync::Arc;

use async_trait::async_trait;

use crate::modules::types::{ModuleSource, ModuleState, ModuleStatus};

use super::super::contract::{DoctorCheck, DoctorCtx, DoctorTier, Finding};
use super::canonical_segment;

pub(super) fn for_context(ctx: &DoctorCtx) -> Vec<Arc<dyn DoctorCheck>> {
    #[cfg(feature = "modules")]
    {
        crate::modules::ops::list(ctx.config())
            .into_iter()
            .map(|status| Arc::new(ModuleStatusCheck::new(status)) as Arc<dyn DoctorCheck>)
            .collect()
    }
    #[cfg(not(feature = "modules"))]
    {
        let _ = ctx;
        Vec::new()
    }
}

struct ModuleStatusCheck {
    id: String,
    status: ModuleStatus,
}

impl ModuleStatusCheck {
    fn new(status: ModuleStatus) -> Self {
        let id = format!("modules/{}", canonical_segment(&status.id));
        Self { id, status }
    }

    fn finding(&self) -> Finding {
        let source = self
            .status
            .source
            .map(module_source)
            .unwrap_or("unavailable");
        let detail = self.status.detail.as_deref().and_then(safe_module_detail);
        match self.status.state {
            ModuleState::Available | ModuleState::Ready => {
                let message = match detail {
                    Some(detail) => format!("Module source is {source}; {detail}."),
                    None => format!("Module is available from the {source} source."),
                };
                Finding::pass(&self.id, message)
            }
            ModuleState::Failed | ModuleState::Unsupported => Finding::error(
                &self.id,
                match detail {
                    Some(detail) => format!(
                        "Module from the {source} source is {}: {detail}.",
                        module_state(self.status.state)
                    ),
                    None => format!(
                        "Module from the {source} source is {}.",
                        module_state(self.status.state)
                    ),
                },
                "Review the module status and restart the core after correcting its installation.",
            ),
            ModuleState::Disabled => Finding::skipped(
                &self.id,
                "Module is disabled by configuration.",
                "Enable the module's owning component to make it available.",
            ),
            ModuleState::Loading => Finding::skipped(
                &self.id,
                "Module is still loading.",
                "Rerun doctor after the module finishes loading.",
            ),
        }
    }
}

/// Reduce module loader details to messages whose contents are known safe for
/// a doctor report. Loader failures can include artifact paths, URLs, and
/// transport diagnostics, so arbitrary detail strings must never reach users.
fn safe_module_detail(detail: &str) -> Option<&'static str> {
    let normalized = detail.trim().to_ascii_lowercase();
    if normalized.contains("digest mismatch") {
        return Some("artifact digest mismatch");
    }
    match normalized.as_str() {
        "no linked factory is registered in this binary; serving the release" => {
            Some("no linked factory is registered in this binary; serving the release")
        }
        "no source-built artifact is available; serving the release" => {
            Some("no source-built artifact is available; serving the release")
        }
        _ => None,
    }
}

#[async_trait]
impl DoctorCheck for ModuleStatusCheck {
    fn id(&self) -> &str {
        &self.id
    }

    fn category(&self) -> &str {
        "modules"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, _ctx: &DoctorCtx) -> Vec<Finding> {
        vec![self.finding()]
    }
}

fn module_source(source: ModuleSource) -> &'static str {
    match source {
        ModuleSource::Release => "release",
        ModuleSource::Linked => "linked",
        ModuleSource::Source => "source",
        ModuleSource::LocalFile => "local file",
    }
}

fn module_state(state: ModuleState) -> &'static str {
    match state {
        ModuleState::Available => "available",
        ModuleState::Loading => "loading",
        ModuleState::Ready => "ready",
        ModuleState::Failed => "failed",
        ModuleState::Unsupported => "unsupported",
        ModuleState::Disabled => "disabled",
    }
}

#[cfg(all(test, feature = "modules"))]
#[path = "modules_tests.rs"]
mod tests;
