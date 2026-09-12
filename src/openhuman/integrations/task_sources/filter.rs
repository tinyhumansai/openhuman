//! Translate a user-configured [`FilterSpec`] into the provider-agnostic
//! [`TaskFetchFilter`] consumed by `ComposioProvider::fetch_tasks`.
//!
//! Each provider's `fetch_tasks` impl reads only the fields relevant to
//! its toolkit, so this mapping just flattens the per-provider
//! `FilterSpec` variant into the shared filter envelope and stamps the
//! per-fetch cap.

use crate::openhuman::integrations::composio::providers::TaskFetchFilter;

use super::types::FilterSpec;

/// Build the runtime [`TaskFetchFilter`] for a source's filter and a
/// per-fetch item cap.
pub fn to_fetch_filter(spec: &FilterSpec, max: u32) -> TaskFetchFilter {
    match spec {
        FilterSpec::Github {
            repo,
            labels,
            assignee_is_me,
            state,
            fetch_mode,
            extra,
        } => TaskFetchFilter {
            assignee_is_me: *assignee_is_me,
            github_fetch_mode: *fetch_mode,
            repo: repo.clone(),
            labels: labels.clone(),
            state: state.clone(),
            extra: extra.clone(),
            max,
            ..Default::default()
        },
        FilterSpec::Notion {
            database_id,
            assigned_to_me,
            status,
            extra,
        } => TaskFetchFilter {
            assignee_is_me: *assigned_to_me,
            database_id: database_id.clone(),
            status: status.clone(),
            extra: extra.clone(),
            max,
            ..Default::default()
        },
        FilterSpec::Linear {
            team_id,
            assignee_is_me,
            state,
            extra,
        } => TaskFetchFilter {
            assignee_is_me: *assignee_is_me,
            team_id: team_id.clone(),
            state: state.clone(),
            extra: extra.clone(),
            max,
            ..Default::default()
        },
        FilterSpec::Clickup {
            team_id,
            list_id,
            assignee_is_me,
            extra,
        } => TaskFetchFilter {
            assignee_is_me: *assignee_is_me,
            team_id: team_id.clone(),
            list_id: list_id.clone(),
            extra: extra.clone(),
            max,
            ..Default::default()
        },
    }
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod tests;
