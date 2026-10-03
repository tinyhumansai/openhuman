//! Provider-backed ops: profile fetch and identity refresh.
//!
//! The connector module reads the connected account (it holds the
//! credential); this host persists the identity it reports through
//! [`super::super::identity_store`]. Syncing records into memory is
//! [`super::sync`]'s.

use crate::config::Config;
use crate::core::Outcome;

use super::super::module_client::{self as connectors, methods};
use super::super::providers::ProviderUserProfile;
use super::super::types::{
    reencode, ComposioRefreshIdentitiesResponse, ComposioUserProfile, ComposioUserProfileRequest,
};
use super::connections::resolve_toolkit_for_connection;
use super::error_utils::{report_composio_op_error, OpResult};

/// Aggregate result of [`composio_refresh_all_identities`].
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefreshIdentitiesReport {
    pub refreshed: usize,
    pub failed: usize,
    pub skipped_no_provider: usize,
    pub skipped_inactive: usize,
    pub rows_written: usize,
}

/// Persist one profile's identity fields and report how many it wrote.
async fn persist_identity(config: &Config, profile: &ComposioUserProfile) -> OpResult<usize> {
    let native: ProviderUserProfile = reencode(profile)?;
    super::super::identity_store::persist_provider_profile(config, &native).await
}

/// `openhuman.composio_get_user_profile` — fetch a normalized user profile for
/// a connected account.
pub async fn composio_get_user_profile(
    config: &Config,
    connection_id: &str,
) -> OpResult<Outcome<ProviderUserProfile>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc get_user_profile");
    let toolkit = resolve_toolkit_for_connection(config, connection_id).await?;

    let profile = connectors::call::<_, ComposioUserProfile>(
        config,
        methods::GET_USER_PROFILE,
        ComposioUserProfileRequest {
            toolkit: toolkit.clone(),
            connection_id: Some(connection_id.to_string()),
        },
    )
    .await
    .map_err(|error| {
        report_composio_op_error("get_user_profile", &anyhow::anyhow!("{error}"));
        format!("[composio] get_user_profile({toolkit}) failed: {error}")
    })?;

    let facets = persist_identity(config, &profile).await?;
    tracing::debug!(
        toolkit = %toolkit,
        facets_written = facets,
        "[composio] persisted identity fields from get_user_profile"
    );

    Ok(Outcome::new(
        reencode(&profile)?,
        vec![format!(
            "composio: fetched {toolkit} profile for connection {connection_id}"
        )],
    ))
}

/// `openhuman.composio_refresh_all_identities` — re-fetch the user profile for
/// every active connection and persist its identity.
///
/// Best-effort per connection: the module reports the ones it could not read as
/// failures alongside the profiles it could, because a refresh exists precisely
/// to find the broken ones.
pub async fn composio_refresh_all_identities(
    config: &Config,
) -> OpResult<Outcome<RefreshIdentitiesReport>> {
    tracing::info!("[composio] rpc refresh_all_identities");
    let response = connectors::call_bare::<ComposioRefreshIdentitiesResponse>(
        config,
        methods::REFRESH_ALL_IDENTITIES,
    )
    .await
    .map_err(|error| {
        report_composio_op_error("refresh_all_identities", &anyhow::anyhow!("{error}"));
        format!("[composio] refresh_all_identities failed: {error}")
    })?;

    let mut report = RefreshIdentitiesReport::default();
    let mut messages: Vec<String> =
        Vec::with_capacity(response.profiles.len() + response.failures.len() + 1);

    for profile in &response.profiles {
        let connection_id = profile.connection_id.as_deref().unwrap_or("-");
        let toolkit = &profile.toolkit;

        // A toolkit the module read but this build has no facet schema for is
        // not a failure — it is the same "no native provider" case the loop
        // used to skip before fetching, now discovered one step later.
        if !super::super::providers::has_native_provider(toolkit) {
            report.skipped_no_provider += 1;
            messages.push(format!(
                "{toolkit}/{connection_id}: skipped (no native provider)"
            ));
            continue;
        }

        let rows = persist_identity(config, profile).await?;
        report.refreshed += 1;
        report.rows_written += rows;
        tracing::debug!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            rows_written = rows,
            "[composio] refresh_all_identities: identity persisted"
        );
        messages.push(format!("{toolkit}/{connection_id}: {rows} row(s)"));
    }

    for failure in &response.failures {
        report.failed += 1;
        tracing::warn!(
            toolkit = %failure.toolkit,
            connection_id = %failure.connection_id,
            error = %failure.message,
            "[composio] refresh_all_identities: fetch_user_profile failed"
        );
        messages.push(format!(
            "{}/{}: ERROR — {}",
            failure.toolkit, failure.connection_id, failure.message
        ));
    }

    let summary = format!(
        "composio: refreshed {ok}/{tried} active conn(s) — {rows} rows; \
         {fail} failed, {nopv} skipped (no provider)",
        ok = report.refreshed,
        tried = report.refreshed + report.failed + report.skipped_no_provider,
        rows = report.rows_written,
        fail = report.failed,
        nopv = report.skipped_no_provider,
    );
    let mut envelope = vec![summary];
    envelope.extend(messages);
    Ok(Outcome::new(report, envelope))
}
