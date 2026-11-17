//! Validated operator-owned inference defaults for hosted profiles.
//!
//! Credentials live only in the existing ephemeral route. Construction rejects
//! partial routes instead of falling back to account or managed inference.

use crate::config::schema::EphemeralRoute;
use serde::{Deserialize, Deserializer};

/// An immutable OpenAI-compatible route and model for every profile turn.
#[derive(Clone, PartialEq, Eq)]
pub struct DirectProfileProvider {
    route: EphemeralRoute,
    model: String,
}

impl std::fmt::Debug for DirectProfileProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectProfileProvider")
            .field("route", &self.route)
            .field("model", &self.model)
            .finish()
    }
}

impl DirectProfileProvider {
    /// Validate a complete compatible endpoint, credential, and model.
    /// HTTPS is required except for loopback HTTP fixtures or local servers.
    ///
    /// # Errors
    /// Rejects blank values, unsafe or invalid endpoints, and invalid bearer values.
    pub fn openai_compatible(
        endpoint: impl Into<String>,
        key: impl Into<String>,
        model: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let endpoint = endpoint.into().trim().to_owned();
        let api_key = key.into().trim().to_owned();
        let model = model.into().trim().to_owned();
        validate_endpoint(&endpoint)?;
        if api_key.is_empty()
            || reqwest::header::HeaderValue::from_str(&format!("Bearer {api_key}")).is_err()
        {
            anyhow::bail!("direct profile provider requires a valid nonblank credential");
        }
        if model.is_empty() || model.chars().any(char::is_control) {
            anyhow::bail!("direct profile provider requires a nonblank model");
        }
        Ok(Self {
            route: EphemeralRoute {
                endpoint,
                api_key,
                headers: Vec::new(),
                ..EphemeralRoute::default()
            },
            model,
        })
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn apply(&self, config: &mut crate::config::Config) {
        config.default_model = Some(self.model.clone());
        crate::config::schema::ephemeral_route::apply(config, self.route.clone());
    }
}

/// Validate destinations without putting their input in an error message.
pub(crate) fn validate_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let parsed = url::Url::parse(endpoint)
        .map_err(|_| anyhow::anyhow!("direct provider endpoint must be a valid absolute URL"))?;
    let loopback = match parsed.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    if parsed.host().is_none()
        || !(parsed.scheme() == "https" || parsed.scheme() == "http" && loopback)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        anyhow::bail!("direct provider endpoint requires HTTPS or loopback HTTP without URL credentials, query, or fragment");
    }
    Ok(())
}

impl<'de> Deserialize<'de> for DirectProfileProvider {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            endpoint: String,
            api_key: String,
            model: String,
        }
        let input = Input::deserialize(deserializer)?;
        Self::openai_compatible(input.endpoint, input.api_key, input.model)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[path = "direct_profile_provider_tests.rs"]
mod tests;
