//! Read-only checks for configured Composio connectivity.

use std::sync::Arc;

use async_trait::async_trait;
use tinyconnectors_bus::{
    names::methods, ComposioConnectedIntegrationsMode, ComposioConnectedIntegrationsResponse,
    ComposioListConnectedIntegrationsRequest,
};

use crate::config::Config;
use crate::doctor::{ComponentId, DoctorCheck, DoctorTier, Finding};

#[async_trait]
trait ComposioProbe: Send + Sync {
    async fn list_existing(&self, config: &Config) -> Result<Option<()>, ()>;
}

struct ExistingConnectorProbe;

#[async_trait]
impl ComposioProbe for ExistingConnectorProbe {
    async fn list_existing(&self, config: &Config) -> Result<Option<()>, ()> {
        #[cfg(feature = "modules")]
        {
            let mode = if config.composio.host_credential.is_some()
                || config.composio.mode.trim() == crate::config::schema::COMPOSIO_MODE_DIRECT
            {
                ComposioConnectedIntegrationsMode::Direct
            } else if config.composio.mode.trim().is_empty()
                || config.composio.mode.trim() == crate::config::schema::COMPOSIO_MODE_BACKEND
            {
                ComposioConnectedIntegrationsMode::Backend
            } else {
                return Ok(None);
            };
            let response = crate::modules::connectors::call_existing_readonly::<
                _,
                ComposioConnectedIntegrationsResponse,
            >(
                config,
                methods::LIST_CONNECTED_INTEGRATIONS,
                ComposioListConnectedIntegrationsRequest { mode },
            )
            .await
            .map_err(|_| ())?;
            return Ok(response.map(|_| ()));
        }
        #[cfg(not(feature = "modules"))]
        {
            let _ = config;
            Ok(None)
        }
    }
}

pub(super) fn for_context() -> Vec<Arc<dyn DoctorCheck>> {
    vec![Arc::new(ComposioCheck {
        probe: Arc::new(ExistingConnectorProbe),
    })]
}

struct ComposioCheck {
    probe: Arc<dyn ComposioProbe>,
}

impl ComposioCheck {
    #[cfg(test)]
    fn with_probe(probe: Arc<dyn ComposioProbe>) -> Self {
        Self { probe }
    }
}

#[async_trait]
impl DoctorCheck for ComposioCheck {
    fn id(&self) -> &str {
        "integrations/composio"
    }
    fn category(&self) -> &str {
        "integrations"
    }
    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }
    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Connectors)
    }

    async fn detect(&self, ctx: &crate::doctor::DoctorCtx) -> Vec<Finding> {
        let finding = if ctx.config().composio.host_credential.is_none()
            && ctx.config().composio.mode.trim() == crate::config::schema::COMPOSIO_MODE_DISABLED
        {
            Finding::skipped(
                self.id(),
                "Composio is disabled in the selected configuration.",
                "Enable Composio in Integrations settings to inspect it.",
            )
        } else {
            match self.probe.list_existing(ctx.config()).await {
                Ok(Some(())) => Finding::pass(self.id(), "The existing Composio connector answered a read-only connection check."),
                Ok(None) => Finding::skipped(self.id(), "No matching ready Composio connector is available for the selected configuration.", "Open Integrations normally, then rerun doctor."),
                Err(()) => Finding::error(self.id(), "The ready Composio connector could not complete its read-only check.", "Check the Composio connection in Integrations, then rerun doctor."),
            }
        };
        vec![finding]
    }
}

#[cfg(test)]
#[path = "integrations_tests.rs"]
mod tests;
