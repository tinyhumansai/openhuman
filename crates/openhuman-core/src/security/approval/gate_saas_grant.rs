// SaaS host authorisation of a tenant's own secret master grant.
//
// A SaaS core serves many tenants, each with its own `.secret_key` scope in
// TinySecurity. Unlocking one needs the module's one-time (HumanOnce) master
// grant, and a headless tenant request (a gateway call, a background or
// delivery job, the embed `profiles` API) has nobody to ask. The host itself
// approves that grant, and only that grant, when the dispatch is bound to an
// authenticated tenant and the scope being unlocked is that tenant's own, as
// the host's profile layout places it. Every other case keeps the gate's
// fail-closed answer, and outside SaaS a person still decides.
//
// The approval runs through the same engine round trip a person's would
// (`check` → `decide`), so TinySecurity verifies and audits it on the tenant's
// own secret scope stream; the decision carries `SAAS_HOST_AUTHORISED` as its
// label and, in the native engine, as its actor.

/// The label (and approving actor) a SaaS host's own approval of a tenant's
/// secret master grant is recorded under in that tenant's approval ledger.
pub(crate) const SAAS_HOST_AUTHORISED: &str = "saas_host_authorised";

/// Whether the SaaS host may approve a forced secret master grant itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SaasHostGrant {
    /// Not a SaaS secret master grant: the gate decides as it always did.
    NotApplicable,
    /// The dispatch's own tenant asks to unlock its own secret scope.
    Authorised { profile: String, scope_id: String },
    /// A SaaS secret master grant the host must not approve.
    Refused(&'static str),
}

/// What the host checks a secret master grant against.
pub(crate) struct SecretGrantClaim<'a> {
    /// The SaaS profile the dispatch is bound to.
    pub tenant_profile: Option<&'a str>,
    /// Where the host's profile layout puts that profile's `.secret_key`.
    pub tenant_key: Option<&'a std::path::Path>,
    /// The live registration the grant would unlock: its `.secret_key` and
    /// its approval scope id.
    pub requested: Option<(&'a std::path::Path, &'a str)>,
}

/// [`saas_host_grant`] over already-resolved facts, so every branch is
/// testable without a SaaS process.
pub(crate) fn saas_host_grant_with(
    saas: bool,
    forced: bool,
    tool_name: &str,
    claim: &SecretGrantClaim<'_>,
) -> SaasHostGrant {
    if !saas || !forced || tool_name != NATIVE_SECRET_GRANT_TOOL {
        return SaasHostGrant::NotApplicable;
    }
    let Some(profile) = claim.tenant_profile.filter(|p| !p.is_empty()) else {
        return SaasHostGrant::Refused("no authenticated tenant scope");
    };
    let Some(tenant_key) = claim.tenant_key else {
        return SaasHostGrant::Refused("the host cannot place this tenant's secret scope");
    };
    let Some((requested_key, scope_id)) = claim.requested else {
        return SaasHostGrant::Refused("the secret scope to unlock is not live");
    };
    if requested_key != tenant_key {
        return SaasHostGrant::Refused("the secret scope is not this tenant's own");
    }
    SaasHostGrant::Authorised {
        profile: profile.to_owned(),
        scope_id: scope_id.to_owned(),
    }
}

/// Whether the SaaS host may approve this forced call itself: resolve the
/// dispatch's tenant, where the profile host keeps that tenant's secret key,
/// and which live secret registration `args.registration_id` names.
pub(crate) fn saas_host_grant(
    ctx: &crate::core::runtime::DispatchCtx,
    saas: bool,
    forced: bool,
    tool_name: &str,
    args: &serde_json::Value,
) -> SaasHostGrant {
    if !saas || !forced || tool_name != NATIVE_SECRET_GRANT_TOOL {
        return SaasHostGrant::NotApplicable;
    }
    let profile = ctx.tenant().ok().and_then(|tenant| tenant.profile);
    let tenant_key = profile
        .as_deref()
        .and_then(|profile| crate::profiles::ProfileId::parse(profile).ok())
        .and_then(|id| {
            let host = crate::profiles::host::host()?;
            let config_path = host.layout_of(&id).config_path;
            Some(
                config_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(".secret_key"),
            )
        });
    let registration_id = args
        .get("registration_id")
        .and_then(serde_json::Value::as_str);
    #[cfg(feature = "security-module")]
    let requested = registration_id
        .and_then(crate::modules::ops::security_tenant_by_registration)
        .map(|tenant| (tenant.key_path.clone(), tenant.scope.scope_id().to_owned()));
    #[cfg(not(feature = "security-module"))]
    let requested: Option<(std::path::PathBuf, String)> = registration_id.and(None);
    saas_host_grant_with(
        saas,
        forced,
        tool_name,
        &SecretGrantClaim {
            tenant_profile: profile.as_deref(),
            tenant_key: tenant_key.as_deref(),
            requested: requested
                .as_ref()
                .map(|(key, scope)| (key.as_path(), scope.as_str())),
        },
    )
}

/// The prompt surface of a host-authorised grant: nobody is shown anything;
/// it only records which request the engine made durable for this call.
#[derive(Default)]
struct HostGrantSurface {
    delivered: Mutex<Option<String>>,
}

#[async_trait::async_trait]
impl PromptSurface for HostGrantSurface {
    async fn deliver(&self, delivery: Delivery) -> Result<(), String> {
        let mut delivered = self.delivered.lock();
        if delivered
            .as_deref()
            .is_some_and(|id| id != delivery.request_id)
        {
            return Err("this host grant already has a request".into());
        }
        *delivered = Some(delivery.request_id);
        Ok(())
    }
}

impl ApprovalGate {
    /// The gate's answer for a SaaS secret master grant nobody can be asked
    /// about, `None` for every other call (including one a person will
    /// decide on a routable WebChat turn).
    #[allow(clippy::too_many_arguments)]
    async fn saas_host_grant_outcome(
        &self,
        dispatch: &crate::core::runtime::DispatchCtx,
        saas: bool,
        human_will_decide: bool,
        forced: bool,
        tool_name: &str,
        action_summary: &str,
        args_redacted: &serde_json::Value,
    ) -> Option<(GateOutcome, Option<String>)> {
        if human_will_decide {
            return None;
        }
        match saas_host_grant(dispatch, saas, forced, tool_name, args_redacted) {
            SaasHostGrant::NotApplicable => None,
            SaasHostGrant::Authorised { profile, scope_id } => Some(
                self.saas_host_authorised_grant(
                    tool_name,
                    action_summary,
                    args_redacted.clone(),
                    &profile,
                    &scope_id,
                )
                .await,
            ),
            SaasHostGrant::Refused(why) => {
                tracing::warn!(why, "[security][saas-grant] host authorisation refused");
                Some((
                    GateOutcome::Deny {
                        reason: format!(
                            "{POLICY_DENIED_MARKER} The host cannot authorise '{tool_name}': {why}."
                        ),
                    },
                    None,
                ))
            }
        }
    }

    /// Approve a SaaS tenant's own secret master grant as the host. The
    /// engine makes the request durable exactly as for a person, then the
    /// host decides it once under [`SAAS_HOST_AUTHORISED`]. Any other answer
    /// from the engine denies.
    async fn saas_host_authorised_grant(
        &self,
        tool_name: &str,
        action_summary: &str,
        args_redacted: serde_json::Value,
        profile: &str,
        scope_id: &str,
    ) -> (GateOutcome, Option<String>) {
        let deny = |why: &str| {
            tracing::warn!(
                tenant = profile,
                scope_id,
                why,
                "[security][saas-grant] host authorisation failed; denying"
            );
            (
                GateOutcome::Deny {
                    reason: format!(
                        "{POLICY_DENIED_MARKER} The host could not authorise '{tool_name}': {why}."
                    ),
                },
                None,
            )
        };
        let surface = Arc::new(HostGrantSurface::default());
        let input = CheckInput {
            tenant: super::gate::current_route_tenant(),
            variant: Variant::Forced,
            posture: Posture {
                require_approval: true,
                ..Posture::default()
            },
            legacy_tool_grants: Vec::new(),
            call_id: fresh_call_id(),
            tool: tool_name.to_string(),
            args: object_arguments(args_redacted),
            summary: action_summary.to_string(),
            thread: None,
            flow: None,
            origin: CallOrigin::Operator,
            ttl: self.effective_ttl(),
            prompt: Arc::clone(&surface) as Arc<dyn PromptSurface>,
        };
        let request_id = match self.engine.check(input).await {
            Ok(Checked::Pending { request_id, .. })
                if surface.delivered.lock().as_deref() == Some(request_id.as_str()) =>
            {
                request_id
            }
            Ok(Checked::Pending { request_id, .. }) | Ok(Checked::Approved { request_id }) => {
                let _ = self.engine.cancel(&request_id).await;
                return deny("the engine did not ask for this grant");
            }
            Err(error) => return deny(&error.to_string()),
        };
        match self
            .engine
            .decide(&request_id, Choice::Once, SAAS_HOST_AUTHORISED)
            .await
        {
            Ok(super::engine::Decided::Approved) => {
                // As for a person's approval of this grant: TinySecurity's
                // ExecuteSecret2 consumes the HumanOnce grant itself.
                self.engine.release(&request_id);
                tracing::info!(
                    tenant = profile,
                    scope_id,
                    request_id = %request_id,
                    "[security][saas-grant] host authorised the tenant's own secret master grant"
                );
                (GateOutcome::Allow, None)
            }
            Ok(super::engine::Decided::Denied) => deny("the engine recorded a denial"),
            Err(error) => {
                let _ = self.engine.cancel(&request_id).await;
                deny(&error.to_string())
            }
        }
    }
}
