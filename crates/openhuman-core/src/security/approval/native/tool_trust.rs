//! Whole-tool trust methods on the native approval engine.

use tinysecurity_bus::{
    names, GrantToolRequest2, ListToolTrustRequest2, RevokeToolByIdRequest2, ToolTrustList2,
    ToolTrustOutcome2, ToolTrustReceipt2,
};

use super::super::super::engine::{EngineError, Posture, Variant};
use super::super::super::types::ToolTrustEntry;
use super::scopes;
use super::{unavailable, NativeEngine, LOG};

impl NativeEngine {
    pub(super) async fn grant_tool_trust(
        &self,
        tenant: &crate::core::runtime::Tenant,
        variant: Variant,
        tool: &str,
    ) -> Result<(), EngineError> {
        if variant != Variant::Standard {
            return Err(unavailable(
                "whole-tool grants are unavailable for strict or forced approval scopes",
            ));
        }
        let proxy = self.proxy().await?;
        let subject = self.subject(tenant, variant)?;
        let scope = scopes::existing(&subject).await.ok_or_else(|| {
            unavailable("the parked request's TinySecurity approval scope is unavailable")
        })?;
        let _guard = scope.stream.lock().await;
        let generation = Self::generation(&proxy).await?;
        let audit = Self::correlation(&scope, generation).await?;
        let receipt: ToolTrustReceipt2 = proxy
            .call_confidential(
                names::methods::GRANT_TOOL2,
                (GrantToolRequest2 {
                    scope_id: scope.scope_id.clone(),
                    tool: tool.to_owned(),
                    audit,
                },),
            )
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "{LOG} the module refused the whole-tool grant");
                unavailable(format!("TinySecurity GrantTool2 failed: {error}"))
            })?;
        if receipt.scope_id != scope.scope_id
            || !matches!(
                receipt.outcome,
                ToolTrustOutcome2::Granted | ToolTrustOutcome2::Unchanged
            )
        {
            return Err(unavailable(
                "TinySecurity returned a mismatched whole-tool grant receipt",
            ));
        }
        tracing::debug!(outcome = ?receipt.outcome, "{LOG} whole-tool grant recorded");
        Ok(())
    }

    pub(super) async fn tool_trust(
        &self,
        tenant: &crate::core::runtime::Tenant,
        variant: Variant,
        posture: Posture,
    ) -> Result<Vec<ToolTrustEntry>, EngineError> {
        if variant != Variant::Standard {
            return Err(unavailable(
                "whole-tool grants are unavailable for strict or forced approval scopes",
            ));
        }
        let proxy = self.proxy().await?;
        let scope = self.tool_scope(&proxy, tenant, &posture).await?;
        let _guard = scope.stream.lock().await;
        let listed: ToolTrustList2 = proxy
            .call_confidential(
                names::methods::LIST_TOOL_TRUST2,
                (ListToolTrustRequest2 {
                    scope_id: scope.scope_id.clone(),
                },),
            )
            .await
            .map_err(|error| unavailable(format!("TinySecurity ListToolTrust2 failed: {error}")))?;
        Ok(listed
            .entries
            .into_iter()
            .map(|entry| ToolTrustEntry {
                tool: entry.tool,
                request_id: entry.request_id,
                granted_at_ms: entry.granted_at_ms,
            })
            .collect())
    }

    pub(super) async fn revoke_tool_trust_by_id(
        &self,
        tenant: &crate::core::runtime::Tenant,
        variant: Variant,
        posture: Posture,
        request_id: &str,
    ) -> Result<bool, EngineError> {
        if variant != Variant::Standard {
            return Err(unavailable(
                "whole-tool grants are unavailable for strict or forced approval scopes",
            ));
        }
        let proxy = self.proxy().await?;
        let scope = self.tool_scope(&proxy, tenant, &posture).await?;
        let _guard = scope.stream.lock().await;
        let generation = Self::generation(&proxy).await?;
        let audit = Self::correlation(&scope, generation).await?;
        let receipt: ToolTrustReceipt2 = proxy
            .call_confidential(
                names::methods::REVOKE_TOOL_BY_ID2,
                (RevokeToolByIdRequest2 {
                    scope_id: scope.scope_id.clone(),
                    request_id: request_id.to_owned(),
                    audit,
                },),
            )
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "{LOG} the module refused to revoke whole-tool trust");
                unavailable(format!("TinySecurity RevokeToolById2 failed: {error}"))
            })?;
        if receipt.scope_id != scope.scope_id
            || !matches!(
                receipt.outcome,
                ToolTrustOutcome2::Revoked | ToolTrustOutcome2::Absent
            )
        {
            return Err(unavailable(
                "TinySecurity returned a mismatched whole-tool revocation receipt",
            ));
        }
        Ok(receipt.outcome == ToolTrustOutcome2::Revoked)
    }
}
