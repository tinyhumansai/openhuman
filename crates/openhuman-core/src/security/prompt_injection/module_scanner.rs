//! The production [`PromptScanner`]: the security module over its bus contract.

use std::sync::Arc;

use async_trait::async_trait;

use super::scanner::{DefinitionScan, PromptScan, PromptScanner};

#[cfg(feature = "security-module")]
use super::scanner::{DefinitionVerdict, Tier};

/// Asks the TinySecurity module (`ScanPrompt2`, `ScanToolDefinition`) under the
/// current dispatch scope.
#[derive(Debug, Default)]
pub(super) struct ModuleScanner;

/// The scanner production uses.
pub(super) fn production() -> Arc<dyn PromptScanner> {
    Arc::new(ModuleScanner)
}

#[cfg(feature = "security-module")]
#[async_trait]
impl PromptScanner for ModuleScanner {
    async fn scan_prompt(&self, text: &str) -> Result<PromptScan, String> {
        let config = crate::core::runtime::CoreContext::current_embedder_config()
            .ok_or("no dispatch scope for the security module")?;
        let response = crate::modules::security::text::scan_prompt_scored(&config, text).await?;
        log::debug!(
            "[security][prompt-scan] module tier={:?} score={} rules={}",
            response.tier,
            response.score_milli,
            response.rule_ids.join(",")
        );
        Ok(PromptScan {
            tier: match response.tier {
                tinysecurity_bus::PromptTier2::Allow => Tier::Allow,
                tinysecurity_bus::PromptTier2::Review => Tier::Review,
                // A tier this host does not know is the strictest one it does.
                _ => Tier::Block,
            },
            score_milli: response.score_milli,
            rule_ids: response.rule_ids,
        })
    }

    async fn scan_tool_definition(
        &self,
        name: &str,
        definition: &serde_json::Value,
    ) -> Result<DefinitionScan, String> {
        use crate::modules::security::text::ScanVerdict;
        let config = crate::core::runtime::CoreContext::current_embedder_config()
            .ok_or("no dispatch scope for the security module")?;
        let response =
            crate::modules::security::text::scan_tool_definition(&config, name, definition).await?;
        Ok(DefinitionScan {
            verdict: match response.verdict {
                ScanVerdict::Clean => DefinitionVerdict::Clean,
                ScanVerdict::Suspicious => DefinitionVerdict::Suspicious,
                _ => DefinitionVerdict::Blocked,
            },
            rule_ids: response.rule_ids,
        })
    }
}

/// A build without the security module has nothing to ask.
#[cfg(not(feature = "security-module"))]
#[async_trait]
impl PromptScanner for ModuleScanner {
    async fn scan_prompt(&self, _text: &str) -> Result<PromptScan, String> {
        Err("the security module is not part of this build".into())
    }

    async fn scan_tool_definition(
        &self,
        _name: &str,
        _definition: &serde_json::Value,
    ) -> Result<DefinitionScan, String> {
        Err("the security module is not part of this build".into())
    }
}
