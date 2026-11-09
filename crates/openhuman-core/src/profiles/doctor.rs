//! SaaS profile selection and read-only inputs for doctor.

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx};
use crate::doctor::{DoctorCheck, DoctorCheckSource, DoctorCtx, DoctorTier, Finding};
use crate::tools::toolpacks::ToolGroups;

use super::host::{self, ProfileHost};
use super::types::ProfileId;

/// The SaaS caller that has already passed the existing dispatch scope guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DoctorCaller {
    Operator,
    Tenant(ProfileId),
}

/// A selected tenant's forced config and no-store diagnostic context.
#[derive(Clone)]
pub(crate) struct TenantDoctorTarget {
    pub profile_id: ProfileId,
    pub config: Config,
    pub context: Arc<CoreContext>,
}

impl std::fmt::Debug for TenantDoctorTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TenantDoctorTarget")
            .field("profile_id", &self.profile_id)
            .finish_non_exhaustive()
    }
}

/// The safe result of reading the tenant registry and lease for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoctorProfileState {
    Unprovisioned,
    Ready,
    HeldElsewhere,
    Unavailable,
}

/// Authorizes a requested tenant before invoking the sole loader for profile
/// registry, config, credential, or lease data.
pub(crate) async fn resolve_with_loader<T, F, Fut>(
    caller: Option<DoctorCaller>,
    requested: Option<&str>,
    load: F,
) -> Result<Option<T>, String>
where
    F: FnOnce(ProfileId) -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let selected = match caller {
        None => return Err("doctor requires an authorized SaaS scope".into()),
        Some(DoctorCaller::Operator) => requested.map(parse_requested_profile).transpose()?,
        Some(DoctorCaller::Tenant(own_profile)) => {
            if let Some(requested) = requested {
                let requested = parse_requested_profile(requested)?;
                if requested != own_profile {
                    return Err("a tenant may only run doctor for its own profile".into());
                }
            }
            Some(own_profile)
        }
    };

    match selected {
        Some(profile_id) => load(profile_id).await.map(Some),
        None => Ok(None),
    }
}

/// Resolves the optional SaaS doctor target. The report handler calls this
/// before loading config; an operator without `profile` keeps the normal
/// operator config path, while a tenant always targets its own profile.
pub(crate) async fn resolve_target(
    dispatch: &DispatchCtx,
    requested: Option<&str>,
) -> Result<Option<TenantDoctorTarget>, String> {
    if !crate::core::runtime::is_saas() {
        return Ok(None);
    }

    let tenant = dispatch.tenant().ok();
    let caller = match (super::surface::current_scope(), tenant) {
        (super::surface::Scope::Operator, Some(tenant)) if tenant.profile.is_none() => {
            Some(DoctorCaller::Operator)
        }
        (super::surface::Scope::User, Some(tenant)) => tenant
            .profile
            .and_then(|profile| ProfileId::parse(&profile).ok().map(DoctorCaller::Tenant)),
        _ => None,
    };

    resolve_with_loader(caller, requested, |profile_id| async move {
        let host =
            host::host().ok_or_else(|| "SaaS profile inspection is unavailable".to_string())?;
        host.target_for_doctor(&profile_id)
    })
    .await
}

fn parse_requested_profile(raw: &str) -> Result<ProfileId, String> {
    ProfileId::parse(raw).map_err(|_| "profile id is invalid".to_string())
}

/// Builds the tenant config and a diagnostic-only context without opening a
/// profile, taking its lease, or creating its workspace.
impl ProfileHost {
    pub(crate) fn target_for_doctor(
        &self,
        profile_id: &ProfileId,
    ) -> Result<TenantDoctorTarget, String> {
        let config = self.serving_config(&self.layout_of(profile_id), profile_id)?;
        let overlay = ContextOverlay::new(
            config.clone(),
            super::host::user_domains(),
            ToolGroups::none(),
        )
        .without_user_skill_roots()
        .profile(profile_id.as_str());
        let context = self.operator.derive_for_inspection(overlay);
        Ok(TenantDoctorTarget {
            profile_id: profile_id.clone(),
            config,
            context,
        })
    }

    /// Reads provision and lease readiness without creating collections or
    /// taking over a profile's lease.
    pub(crate) async fn inspect_for_doctor(&self, profile_id: &ProfileId) -> DoctorProfileState {
        match self.registry.get_readonly(profile_id).await {
            Ok(Some(_)) => {}
            Ok(None) => return DoctorProfileState::Unprovisioned,
            Err(_) => return DoctorProfileState::Unavailable,
        }

        let now = super::lease::now_ms();
        match self.leases.holder_readonly(profile_id.as_str()).await {
            Ok(Some(record)) if record.is_live(now) && record.owner != self.node => {
                DoctorProfileState::HeldElsewhere
            }
            Ok(_) => DoctorProfileState::Ready,
            Err(_) => DoctorProfileState::Unavailable,
        }
    }
}

/// Emits the safe operator-facing result for `profile/provisioned`.
pub(crate) fn profile_provisioned_finding(state: DoctorProfileState) -> Finding {
    match state {
        DoctorProfileState::Ready => Finding::pass(
            "profile/provisioned",
            "The tenant profile is provisioned and available on this node.",
        ),
        DoctorProfileState::Unprovisioned => Finding::error(
            "profile/provisioned",
            "The tenant profile is not provisioned.",
            "Provision the tenant profile through the normal profile management flow, then rerun doctor.",
        ),
        DoctorProfileState::HeldElsewhere => Finding::error(
            "profile/provisioned",
            "The tenant profile is currently held by another node.",
            "Run doctor on the node holding this profile, or retry after its lease is released.",
        ),
        DoctorProfileState::Unavailable => Finding::error(
            "profile/provisioned",
            "The tenant profile registry or lease could not be verified safely.",
            "Check profile storage and complete any required storage migration before rerunning doctor.",
        ),
    }
}

struct ProfileProvisionedCheck {
    host: Arc<ProfileHost>,
}

#[async_trait]
impl DoctorCheck for ProfileProvisionedCheck {
    fn id(&self) -> &str {
        "profile/provisioned"
    }

    fn category(&self) -> &str {
        "Profile"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let Some(profile) = ctx.profile() else {
            return vec![Finding::skipped(
                self.id(),
                "No tenant profile was selected for this report.",
                "Pass a tenant profile id to inspect SaaS profile readiness.",
            )];
        };
        let state = match ProfileId::parse(profile) {
            Ok(profile_id) => self.host.inspect_for_doctor(&profile_id).await,
            Err(_) => DoctorProfileState::Unavailable,
        };
        vec![profile_provisioned_finding(state)]
    }
}

struct ProfileDoctorCheckSource {
    host: Arc<ProfileHost>,
}

impl DoctorCheckSource for ProfileDoctorCheckSource {
    fn id(&self) -> &str {
        "profiles-doctor"
    }

    fn checks(&self) -> Vec<Arc<dyn DoctorCheck>> {
        vec![Arc::new(ProfileProvisionedCheck {
            host: Arc::clone(&self.host),
        })]
    }
}

pub(crate) fn register_check_source(host: Arc<ProfileHost>) {
    crate::doctor::register_check_source(ProfileDoctorCheckSource { host });
}

#[cfg(test)]
#[path = "doctor_tests.rs"]
mod tests;
