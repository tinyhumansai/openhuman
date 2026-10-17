use super::*;

fn only(read: bool, write: bool, admin: bool) -> UserScopePref {
    UserScopePref { read, write, admin }
}

#[test]
fn a_curated_write_action_follows_the_write_flag() {
    let allowed = decide_visibility("gmail", "GMAIL_SEND_EMAIL", only(true, true, false));
    assert!(matches!(allowed, ToolDecision::Allow));
    let blocked = decide_visibility("gmail", "GMAIL_SEND_EMAIL", only(true, false, false));
    assert!(matches!(
        blocked,
        ToolDecision::BlockedByScope {
            scope: ToolScope::Write
        }
    ));
}

#[test]
fn an_action_outside_a_curated_catalog_is_not_curated() {
    let decision = decide_visibility("gmail", "GMAIL_NOT_A_REAL_ACTION", UserScopePref::default());
    assert!(matches!(decision, ToolDecision::NotCurated));
}

#[test]
fn an_uncatalogued_toolkit_falls_back_to_the_verb_heuristic() {
    let read = decide_visibility(
        "flowstestkit",
        "FLOWSTESTKIT_GET_THING",
        only(true, false, false),
    );
    assert!(matches!(read, ToolDecision::PassthroughCheckScope { .. }));
    let admin = decide_visibility(
        "flowstestkit",
        "FLOWSTESTKIT_DELETE_THING",
        only(true, true, false),
    );
    assert!(matches!(
        admin,
        ToolDecision::BlockedByScope {
            scope: ToolScope::Admin
        }
    ));
}

#[test]
fn unknown_slugs_default_to_write_for_the_sandbox_gate() {
    assert_eq!(resolve_action_scope_sync("  "), ToolScope::Write);
    assert!(action_mutates_external_state("   "));
}

#[test]
fn the_scope_error_names_the_toolkit_and_the_unlock_path() {
    let message = scope_error_message(
        "GMAIL_SEND_EMAIL",
        ToolScope::Write,
        only(true, false, false),
    );
    assert!(message.contains("Connections → gmail → write"), "{message}");
    assert!(message.contains("write=false"), "{message}");
}

fn visible(
    toolkit: &str,
    has_catalog: bool,
    slugs: &[&str],
    pref: UserScopePref,
) -> VisibleToolsResponse {
    VisibleToolsResponse {
        toolkit: toolkit.to_owned(),
        preference: pref,
        has_catalog,
        tools: slugs
            .iter()
            .map(|slug| tinyconnectors_bus::vocabulary::CuratedTool {
                toolkit: toolkit.to_owned(),
                slug: (*slug).to_owned(),
                scope: ToolScope::Write,
            })
            .collect(),
    }
}

#[test]
fn a_slug_in_the_modules_visible_list_is_allowed_regardless_of_case() {
    let listing = visible(
        "gmail",
        true,
        &["GMAIL_SEND_EMAIL"],
        UserScopePref::default(),
    );
    assert!(matches!(
        decide_from_visible("gmail_send_email", &listing),
        ToolDecision::Allow
    ));
}

#[test]
fn a_curated_slug_missing_from_the_visible_list_is_blocked_by_its_scope() {
    // The module withheld it: the catalog has it, the preference does not allow it.
    let listing = visible("gmail", true, &[], only(true, false, false));
    assert!(matches!(
        decide_from_visible("GMAIL_SEND_EMAIL", &listing),
        ToolDecision::BlockedByScope {
            scope: ToolScope::Write
        }
    ));
}

#[test]
fn a_slug_the_catalog_never_names_is_not_curated() {
    let listing = visible("gmail", true, &[], UserScopePref::default());
    assert!(matches!(
        decide_from_visible("GMAIL_NOT_A_REAL_ACTION", &listing),
        ToolDecision::NotCurated
    ));
}

#[test]
fn an_uncatalogued_toolkit_is_gated_by_the_preference_the_module_applied() {
    let listing = visible("flowstestkit", false, &[], only(true, false, false));
    assert!(matches!(
        decide_from_visible("FLOWSTESTKIT_GET_THING", &listing),
        ToolDecision::PassthroughCheckScope { .. }
    ));
    assert!(matches!(
        decide_from_visible("FLOWSTESTKIT_SEND_THING", &listing),
        ToolDecision::BlockedByScope {
            scope: ToolScope::Write
        }
    ));
}

#[test]
fn the_slack_channel_invite_stays_admin_scoped() {
    // Regression guard for the connector move: the host classified this `Write`
    // before the catalogs moved to the module, which decided it is `Admin`
    // (adding someone to a channel hands them its history). The default
    // preference, which has no admin, must not offer it.
    let catalog = catalog_for_toolkit("slack").expect("slack is curated");
    let invite = find_curated(&catalog, "SLACK_INVITE_USERS_TO_A_SLACK_CHANNEL")
        .expect("the invite is curated");
    assert_eq!(invite.scope, ToolScope::Admin);
    assert!(!UserScopePref::default().allows(invite.scope));
}

#[tokio::test]
async fn a_preference_still_in_the_legacy_file_gates_execution() {
    // The execute gate asks the module (`ListVisibleTools`), which only knows
    // rows already migrated out of the legacy workspace file.
    use crate::integrations::composio::module_client::module_guard;
    let _serialised = module_guard().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    crate::integrations::composio::file_store::save(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        crate::integrations::composio::file_store::USER_SCOPES_FILE,
        &serde_json::json!({"gmail": {"read": true, "write": false, "admin": false}}),
    )
    .await
    .unwrap();

    let decision = evaluate_tool_visibility(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "GMAIL_SEND_EMAIL",
    )
    .await;
    let restored = crate::integrations::composio::ops::save_user_scope_pref(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "gmail",
        Default::default(),
    )
    .await;
    restored.unwrap();

    assert!(matches!(
        decision,
        ToolDecision::BlockedByScope {
            scope: ToolScope::Write
        }
    ));
}
