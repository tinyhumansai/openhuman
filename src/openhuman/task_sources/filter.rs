//! Translate a user-configured [`FilterSpec`] into the provider-agnostic
//! [`TaskFetchFilter`] consumed by `ComposioProvider::fetch_tasks`.
//!
//! Each provider's `fetch_tasks` impl reads only the fields relevant to
//! its toolkit, so this mapping just flattens the per-provider
//! `FilterSpec` variant into the shared filter envelope and stamps the
//! per-fetch cap.

use crate::openhuman::memory_sync::composio::providers::TaskFetchFilter;

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
            extra,
        } => TaskFetchFilter {
            assignee_is_me: *assignee_is_me,
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn github_filter_maps_repo_labels_and_assignee() {
        let spec = FilterSpec::Github {
            repo: Some("tinyhumansai/openhuman".into()),
            labels: vec!["bug".into(), "p1".into()],
            assignee_is_me: true,
            state: Some("open".into()),
            extra: json!({"per_page": 10}),
        };
        let f = to_fetch_filter(&spec, 25);
        assert_eq!(f.repo.as_deref(), Some("tinyhumansai/openhuman"));
        assert_eq!(f.labels, vec!["bug".to_string(), "p1".to_string()]);
        assert!(f.assignee_is_me);
        assert_eq!(f.state.as_deref(), Some("open"));
        assert_eq!(f.extra["per_page"], 10);
        assert_eq!(f.max, 25);
        // Fields that don't apply to github stay unset.
        assert!(f.database_id.is_none());
        assert!(f.team_id.is_none());
    }

    #[test]
    fn notion_filter_maps_board_and_status() {
        let spec = FilterSpec::Notion {
            database_id: Some("board-xyz".into()),
            assigned_to_me: true,
            status: Some("Todo".into()),
            extra: json!({}),
        };
        let f = to_fetch_filter(&spec, 5);
        assert_eq!(f.database_id.as_deref(), Some("board-xyz"));
        assert!(f.assignee_is_me, "assigned_to_me maps to assignee_is_me");
        assert_eq!(f.status.as_deref(), Some("Todo"));
        assert_eq!(f.max, 5);
    }

    #[test]
    fn linear_filter_maps_team_and_state() {
        let spec = FilterSpec::Linear {
            team_id: Some("team-1".into()),
            assignee_is_me: true,
            state: Some("started".into()),
            extra: json!({}),
        };
        let f = to_fetch_filter(&spec, 50);
        assert_eq!(f.team_id.as_deref(), Some("team-1"));
        assert!(f.assignee_is_me);
        assert_eq!(f.state.as_deref(), Some("started"));
    }

    #[test]
    fn clickup_filter_maps_team_and_list() {
        let spec = FilterSpec::Clickup {
            team_id: Some("ws-1".into()),
            list_id: Some("list-9".into()),
            assignee_is_me: true,
            extra: json!({}),
        };
        let f = to_fetch_filter(&spec, 25);
        assert_eq!(f.team_id.as_deref(), Some("ws-1"));
        assert_eq!(f.list_id.as_deref(), Some("list-9"));
        assert!(f.assignee_is_me);
    }
}
