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
//
// The authorisation rides the store's `CoreContext` as a [`HostSecretGrant`].
// It holds only on the host's own task and only until the store ends, so a
// task spawned from inside the scope, or a context captured there and used
// later, authorises nothing.

/// The label (and approving actor) an embedder's own approval of its
/// runtime's secret master grant is recorded under.
pub(crate) const EMBEDDER_HOST_AUTHORISED: &str = "embedder_host_authorised";

/// A library host's authorisation of the secret master grant of its own
/// runtime's config directory, carried on the context of the store it scopes
/// ([`host_authorised_secret_grant`]).
pub(crate) struct HostSecretGrant {
    /// The `.secret_key` the host authorises.
    key: std::path::PathBuf,
    /// The runtime config owning that key.
    config: Config,
    /// The host's own task: the only one the grant holds on.
    task: Option<tokio::task::Id>,
    /// Cleared when the store the host scoped ends.
    live: AtomicBool,
}

impl HostSecretGrant {
    /// Whether the grant holds for work running here, now.
    fn holds(&self) -> bool {
        self.live.load(Ordering::Acquire) && tokio::task::try_id() == self.task
    }
}

/// Ends a host grant when the store it authorises ends, however it ends.
struct HostGrantEnds(Arc<HostSecretGrant>);

impl Drop for HostGrantEnds {
    fn drop(&mut self) {
        self.0.live.store(false, Ordering::Release);
    }
}

/// The host grant in force for work running here, if any.
fn host_secret_grant() -> Option<Arc<HostSecretGrant>> {
    crate::core::runtime::CoreContext::current_host_secret_grant().filter(|grant| grant.holds())
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
    let grant = Arc::new(HostSecretGrant {
        key,
        config: config.clone(),
        task: tokio::task::try_id(),
        live: AtomicBool::new(true),
    });
    let _ends = HostGrantEnds(Arc::clone(&grant));
    crate::core::runtime::CoreContext::scope_with_host_secret_grant(grant, fut).await
}

/// The approval label of a host-authorised grant made here.
fn host_grant_label() -> &'static str {
    if host_secret_grant().is_some() && !crate::core::runtime::is_saas() {
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
    let Some(grant) = host_secret_grant() else {
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
            tenant_key: Some(&grant.key),
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
    let config = host_secret_grant()?.config.clone();
    match embedder_host_grant(true, tool_name, &args) {
        SaasHostGrant::Authorised { profile, scope_id } => {
            let session = format!("session-{}", uuid::Uuid::new_v4());
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
