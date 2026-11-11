// An embedder's authorisation of its own runtime's secret master grant.
//
// A library host that hands its runtime a credential (the embed
// `RuntimeBuilder::api_key`) is the authority over that runtime's config
// directory. Storing the credential needs TinySecurity's HumanOnce master
// grant, and there is nobody to ask: no chat, and before the core boots not
// even a global gate. The host scopes the store with
// [`host_authorised_secret_grant`]. Inside that scope, and only for the secret
// scope of that exact config directory, the grant is approved through the same
// engine round trip a person's approval takes (`saas_host_authorised_grant`),
// recorded under [`EMBEDDER_HOST_AUTHORISED`]. Outside the scope nothing
// changes, and inside it a grant for any other scope is refused.

/// The label (and approving actor) an embedder's own approval of its
/// runtime's secret master grant is recorded under.
pub(crate) const EMBEDDER_HOST_AUTHORISED: &str = "embedder_host_authorised";

tokio::task_local! {
    /// The `.secret_key` the host authorises, with the runtime config owning it.
    static HOST_AUTHORISED_SECRET: (std::path::PathBuf, Config);
}

/// Run `fut` with the host authorising the secret master grant of `config`'s
/// own config directory, and of no other. For a library host storing the
/// credential it supplied itself; never for work a model or a remote caller
/// started.
pub async fn host_authorised_secret_grant<F: std::future::Future>(
    config: &Config,
    fut: F,
) -> F::Output {
    let key = config
        .config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(".secret_key");
    tracing::debug!("[security][host-grant] host authorises its own runtime's secret grant");
    HOST_AUTHORISED_SECRET
        .scope((key, config.clone()), fut)
        .await
}

/// The approval label of a host-authorised grant in this task.
fn host_grant_label() -> &'static str {
    if HOST_AUTHORISED_SECRET.try_with(|_| ()).is_ok() && !crate::core::runtime::is_saas() {
        EMBEDDER_HOST_AUTHORISED
    } else {
        SAAS_HOST_AUTHORISED
    }
}

/// Whether the host authorised the forced grant `args.registration_id` names:
/// not applicable outside [`host_authorised_secret_grant`], authorised only
/// for the scoped config directory's own live secret scope.
pub(crate) fn embedder_host_grant(
    forced: bool,
    tool_name: &str,
    args: &serde_json::Value,
) -> SaasHostGrant {
    let Ok(key) = HOST_AUTHORISED_SECRET.try_with(|(key, _)| key.clone()) else {
        return SaasHostGrant::NotApplicable;
    };
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
        true,
        forced,
        tool_name,
        &SecretGrantClaim {
            tenant_profile: Some("embedder"),
            tenant_key: Some(&key),
            requested: requested
                .as_ref()
                .map(|(key, scope)| (key.as_path(), scope.as_str())),
        },
    )
}

/// The host's answer for a secret master grant asked before any global gate
/// exists (the embed runtime storing its API key ahead of boot): approved
/// through a private gate over the real engine when
/// [`embedder_host_grant`] authorises it, `None` otherwise.
pub(crate) async fn host_authorised_grant_without_gate(
    tool_name: &str,
    action_summary: &str,
    args: serde_json::Value,
) -> Option<GateOutcome> {
    let config = HOST_AUTHORISED_SECRET
        .try_with(|(_, config)| config.clone())
        .ok()?;
    match embedder_host_grant(true, tool_name, &args) {
        SaasHostGrant::Authorised { profile, scope_id } => {
            let session = format!("host-grant-{}", uuid::Uuid::new_v4());
            let gate = ApprovalGate::new(config, session, None);
            let (outcome, _) = gate
                .saas_host_authorised_grant(tool_name, action_summary, args, &profile, &scope_id)
                .await;
            Some(outcome)
        }
        SaasHostGrant::Refused(why) => {
            tracing::warn!(why, "[security][host-grant] host authorisation refused");
            Some(GateOutcome::Deny {
                reason: format!(
                    "{POLICY_DENIED_MARKER} The host cannot authorise '{tool_name}': {why}."
                ),
            })
        }
        SaasHostGrant::NotApplicable => None,
    }
}
