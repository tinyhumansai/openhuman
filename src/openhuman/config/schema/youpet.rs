//! YouPet Core integration configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

const DEFAULT_YOUPET_CORE_API_URL: &str = "http://127.0.0.1:8000";
const DEFAULT_YOUPET_WORKBENCH_ACTOR_ID: &str = "openhuman-workbench";

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct YouPetConfig {
    /// YouPet Core REST API base URL.
    pub core_api_url: String,
    /// Service-token credential for Core service-to-service routes.
    pub service_token: Option<String>,
    /// Actor label sent in X-Actor-Id for workbench operations.
    pub workbench_actor_id: String,
    /// Core users.id UUID for the operator performing workbench actions.
    pub operator_user_id: Option<String>,
    /// Default Core tenant UUID for ActionRequest list scope.
    pub tenant_id: Option<String>,
}

impl fmt::Debug for YouPetConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("YouPetConfig")
            .field("core_api_url", &self.core_api_url)
            .field(
                "service_token",
                &self
                    .service_token
                    .as_deref()
                    .filter(|token| !token.trim().is_empty())
                    .map(|_| "<redacted>"),
            )
            .field("workbench_actor_id", &self.workbench_actor_id)
            .field("operator_user_id", &self.operator_user_id)
            .field("tenant_id", &self.tenant_id)
            .finish()
    }
}

impl Default for YouPetConfig {
    fn default() -> Self {
        Self {
            core_api_url: DEFAULT_YOUPET_CORE_API_URL.to_string(),
            service_token: None,
            workbench_actor_id: DEFAULT_YOUPET_WORKBENCH_ACTOR_ID.to_string(),
            operator_user_id: None,
            tenant_id: None,
        }
    }
}

impl YouPetConfig {
    pub fn normalized_core_api_url(&self) -> String {
        let trimmed = self.core_api_url.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            DEFAULT_YOUPET_CORE_API_URL.to_string()
        } else {
            trimmed.to_string()
        }
    }

    pub fn service_token(&self) -> Option<&str> {
        self.service_token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
    }

    pub fn workbench_actor_id(&self) -> &str {
        let trimmed = self.workbench_actor_id.trim();
        if trimmed.is_empty() {
            DEFAULT_YOUPET_WORKBENCH_ACTOR_ID
        } else {
            trimmed
        }
    }

    pub fn operator_user_id(&self) -> Option<&str> {
        self.operator_user_id
            .as_deref()
            .map(str::trim)
            .filter(|operator| !operator.is_empty())
    }

    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|tenant| !tenant.is_empty())
    }
}

#[cfg(test)]
#[path = "youpet_config_tests.rs"]
mod tests;
