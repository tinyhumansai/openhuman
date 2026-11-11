// SaaS host authorisation of a tenant's own secret master grant.

/// The label (and approving actor) a SaaS host's own approval is recorded under.
pub(crate) const SAAS_HOST_AUTHORISED: &str = "saas_host_authorised";

/// Whether the SaaS host may approve a forced secret master grant itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SaasHostGrant {
    NotApplicable,
    Authorised { profile: String, scope_id: String },
    Refused(&'static str),
}

/// What the host checks a secret master grant against.
pub(crate) struct SecretGrantClaim<'a> {
    pub tenant_profile: Option<&'a str>,
    pub tenant_key: Option<&'a std::path::Path>,
    pub requested: Option<(&'a std::path::Path, &'a str)>,
}

pub(crate) fn saas_host_grant_with(
    _saas: bool,
    _forced: bool,
    _tool_name: &str,
    _claim: &SecretGrantClaim<'_>,
) -> SaasHostGrant {
    SaasHostGrant::NotApplicable
}

pub(crate) fn saas_host_grant(
    _ctx: &crate::core::runtime::DispatchCtx,
    _saas: bool,
    _forced: bool,
    _tool_name: &str,
    _args: &serde_json::Value,
) -> SaasHostGrant {
    SaasHostGrant::NotApplicable
}

impl ApprovalGate {
    async fn saas_host_authorised_grant(
        &self,
        tool_name: &str,
        _action_summary: &str,
        _args_redacted: serde_json::Value,
        _profile: &str,
        _scope_id: &str,
    ) -> (GateOutcome, Option<String>) {
        (
            GateOutcome::Deny {
                reason: format!("{POLICY_DENIED_MARKER} '{tool_name}' not implemented"),
            },
            None,
        )
    }
}
