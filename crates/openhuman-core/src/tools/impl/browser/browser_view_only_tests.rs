use super::*;
use serde_json::json;

/// Every action that can change a page or act for the signed-in user.
const REFUSED: &[&str] = &[
    "click",
    "double_click",
    "fill",
    "type",
    "press",
    "select",
    "check",
    "task",
    "task_continue",
    "confirm_pending",
    "wait_download",
    "task_cancel",
];

/// Reading, looking and leaving.
const ALLOWED: &[&str] = &[
    "open",
    "snapshot",
    "read_page",
    "get_text",
    "get_title",
    "get_url",
    "wait",
    "hover",
    "scroll",
    "is_visible",
    "list_downloads",
    "close",
];

#[test]
fn every_acting_action_is_refused_by_name() {
    for action in REFUSED {
        let refusal = view_only_refusal(action, &json!({"action": action}))
            .unwrap_or_else(|| panic!("{action} must be refused"));
        assert!(
            refusal.contains(&format!("`{action}`")),
            "{action}: {refusal}"
        );
        assert!(refusal.contains("view-only"), "{action}: {refusal}");
    }
}

#[test]
fn every_reading_action_passes() {
    for action in ALLOWED {
        assert_eq!(
            view_only_refusal(action, &json!({"action": action})),
            None,
            "{action}"
        );
    }
}

#[test]
fn find_passes_only_when_it_reads_or_hovers() {
    for follow in [
        json!("click"),
        json!("fill"),
        json!(" Click "),
        json!("TEXT"),
        json!("press"),
        json!(null),
    ] {
        let args = json!({"action": "find", "by": "text", "value": "Go", "find_action": follow});
        let refusal = view_only_refusal("find", &args)
            .unwrap_or_else(|| panic!("find with {follow} must be refused"));
        assert!(refusal.contains("`find`"), "{refusal}");
        assert!(refusal.contains("view-only"), "{refusal}");
    }
    for follow in ["text", "hover"] {
        let args = json!({"action": "find", "by": "text", "value": "Go", "find_action": follow});
        assert_eq!(view_only_refusal("find", &args), None, "{follow}");
    }
    // The tool requires `find_action`; without one there is nothing to allow.
    assert!(view_only_refusal(
        "find",
        &json!({"action": "find", "by": "text", "value": "Go"})
    )
    .is_some());
}

#[test]
fn an_action_the_list_does_not_know_is_refused() {
    // A new acting action must not become available to a view-only run just
    // because nobody added it to a deny list.
    for action in ["drag", "", "evaluate", "upload", "Open", " open"] {
        assert!(
            view_only_refusal(action, &json!({"action": action})).is_some(),
            "{action:?}"
        );
    }
}

#[test]
fn a_view_only_session_never_shares_a_pool_key_with_a_normal_one() {
    let scope = Path::new("/srv/agents/teeny");
    assert_eq!(
        pool_key(scope, "thread-1", false),
        "/srv/agents/teeny:thread-1",
        "the normal key is unchanged"
    );
    let view_only = pool_key(scope, "thread-1", true);
    assert_ne!(view_only, pool_key(scope, "thread-1", false));
    assert_ne!(view_only, pool_key(scope, "thread-1:view-only", false));
    assert_ne!(view_only, pool_key(scope, "view-only:thread-1", false));
    assert_ne!(view_only, pool_key(scope, "thread-2", true));
}

mod tool {
    use super::super::super::session_pool::{
        browser_session_fingerprint, thread_sessions, ThreadSession,
    };
    use super::super::super::BrowserTool;
    use super::*;
    use crate::modules::browser::BrowserClient;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tinycomputer_bus::browser::SessionId;
    use tinytools::Tool;

    const PROFILE: &str = "/home/host/.config/signed-in-chrome";

    fn persistent_client() -> Arc<BrowserClient> {
        let mut config = crate::config::Config::default();
        // No module and no download: a call that reached the module fails fast.
        config.modules.enabled = false;
        config.modules.allow_download = false;
        config.browser.profile_mode = "persistent".into();
        config.browser.profile_path = Some(PROFILE.into());
        Arc::new(BrowserClient::new(Arc::new(config)))
    }

    #[test]
    fn a_view_only_run_gets_a_fresh_profile_and_a_normal_run_the_persistent_one() {
        let tool = BrowserTool::new(persistent_client(), 3);
        assert_eq!(
            tool.for_run(false).client.session_profile().as_deref(),
            Some(PROFILE)
        );
        assert_eq!(tool.for_run(true).client.session_profile(), None);
    }

    /// A view-only run in a conversation that already holds a browser on the
    /// persistent profile neither picks that session up nor touches it: it
    /// looks under its own key.
    #[tokio::test]
    async fn a_view_only_run_never_picks_up_the_conversations_persistent_session() {
        let client = persistent_client();
        let thread = format!("view-only-pool-{}", uuid::Uuid::new_v4());
        let scope = crate::core::runtime::agent_scope_dir(client.config());
        let normal_key = pool_key(&scope, &thread, false);
        let earlier = Instant::now() - Duration::from_secs(5);
        thread_sessions().lock().await.insert(
            normal_key.clone(),
            ThreadSession {
                id: SessionId::new("persistent-profile-browser"),
                config_fingerprint: browser_session_fingerprint(&client),
                client: client.clone(),
                last_used: earlier,
                bound_origin: None,
                password_typed: false,
            },
        );
        let tool = BrowserTool::new(client, 3);
        let mut host = crate::agent::tinyagents::host::OpenHumanRunContext::new();
        host.browser_view_only = true;
        let run = host.into_tinyagents(
            tinyagents_harness::context::RunConfig::new("view-only-pool").with_thread(&thread),
        );
        let call = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
            &run,
            tinyagents_harness::ids::CallId::new("view-only-pool-call"),
        );
        let result = tool
            .execute_with_context(
                json!({"action": "get_url"}),
                tinytools::ToolCallOptions::default(),
                Some(&call),
            )
            .await
            .unwrap();
        let untouched = thread_sessions()
            .lock()
            .await
            .remove(&normal_key)
            .map(|entry| (entry.id, entry.last_used));
        thread_sessions()
            .lock()
            .await
            .remove(&pool_key(&scope, &thread, true));
        // Opening its own fresh-profile session needs the module this test
        // does not load.
        assert!(result.is_error, "{}", result.output());
        // The pool is process-wide: a sibling test filling it may evict the
        // entry for capacity, which is no use of it either.
        if let Some(entry) = untouched {
            assert_eq!(
                entry,
                (SessionId::new("persistent-profile-browser"), earlier),
                "the persistent-profile session was neither used nor replaced"
            );
        }
    }
}
