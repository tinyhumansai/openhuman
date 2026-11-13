use super::*;

fn thread_with_delivered_proposal(name: &str) -> tempfile::TempDir {
    thread_with_delivered_proposal_extra(name, "")
}

/// Deliver a proposal whose payload carries extra fields (e.g.
/// `,"draft_id":"draft-old"`), for hand-offs that identify the proposal by
/// id rather than by name.
fn thread_with_delivered_proposal_extra(name: &str, extra: &str) -> tempfile::TempDir {
    use crate::threads::store::CreateConversationThread;
    let temp = tempfile::tempdir().expect("tempdir");
    conversations::ensure_thread(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path().to_path_buf(),
        CreateConversationThread {
            id: "thread-parent".into(),
            title: "Main chat".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .expect("thread created");
    attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-propose",
        "workflow_builder",
        &[TranscriptMessage::tool(format!(
            r#"{{"type":"workflow_proposal","persisted":false,"name":"{name}","graph":{{"nodes":[],"edges":[]}}{extra}}}"#
        ))],
        "proposal".to_string(),
    );
    temp
}

/// The card's save, as `WorkflowProposalCard` performs it: record
/// `saved_flow_id` on the delivered proposal message.
fn mark_proposal_saved_from_card(temp: &tempfile::TempDir, flow_id: &str) {
    let patch = conversations::ConversationMessagePatch {
        merge_extra_metadata: Some(json!({ "saved_flow_id": flow_id })),
        ..Default::default()
    };
    conversations::update_message(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path().to_path_buf(),
        "thread-parent",
        "workflow-proposal:sub-propose",
        patch,
    )
    .expect("mark saved");
}

#[test]
fn a_card_saved_proposal_is_found_by_the_lookup() {
    let temp = thread_with_delivered_proposal("Friday Inbox Wrap-up");
    mark_proposal_saved_from_card(&temp, "flow-1");
    let found = already_saved_workflow_proposals(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        "thread-parent",
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message_id, "workflow-proposal:sub-propose");
    assert_eq!(found[0].name, "Friday Inbox Wrap-up");
    assert_eq!(found[0].saved_flow_id.as_deref(), Some("flow-1"));
}

#[test]
fn a_proposal_without_a_saved_flow_is_not_found() {
    let temp = thread_with_delivered_proposal("Friday Inbox Wrap-up");
    assert!(already_saved_workflow_proposals(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        "thread-parent"
    )
    .is_empty());
}

#[test]
fn the_saved_note_names_the_proposal_and_forbids_create_workflow() {
    let note = already_saved_proposal_note("Friday Inbox Wrap-up", "flow-1");
    assert!(note.contains("Friday Inbox Wrap-up"));
    assert!(note.contains("flow-1"));
    assert!(note.contains("Do not call `create_workflow`"));
    assert!(note.contains("already saved"));
    assert!(note.contains("save_workflow"));
    assert!(note.contains("separate variant or copy"));
}

#[test]
fn carry_saved_flow_appends_the_note_only_for_the_named_proposal() {
    // The user saved the proposal from its card, then said "save it" in chat:
    // the hand-off for that proposal must carry the note, and a hand-off for
    // a different proposal must not.
    let temp = thread_with_delivered_proposal("Friday Inbox Wrap-up");
    mark_proposal_saved_from_card(&temp, "flow-1");
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let thread = Some("thread-parent".to_string());
    let save_it = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save the delivered proposal \"Friday Inbox Wrap-up\" as-is.".to_string(),
        None,
    );
    assert!(save_it.contains("flow-1"));
    assert!(save_it.contains("already saved"));
    assert!(save_it.contains("Do not call `create_workflow`"));
    let other = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save the delivered proposal \"Something Else\" as-is.".to_string(),
        None,
    );
    assert_eq!(
        other,
        "Save the delivered proposal \"Something Else\" as-is."
    );
    let no_thread: Option<String> = None;
    let unthreaded = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &no_thread,
        "Save the delivered proposal \"Friday Inbox Wrap-up\" as-is.".to_string(),
        None,
    );
    assert_eq!(
        unthreaded,
        "Save the delivered proposal \"Friday Inbox Wrap-up\" as-is."
    );
}

/// The live spawn path (`carry_saved_flow`) runs the thread read and the
/// `flows.get` presence probe off the async worker. From a multi-threaded
/// runtime it must still find the saved proposal and append a note, without a
/// nested-runtime panic. `flow-1` is in no flow catalog here, so the probe
/// answers `Missing` (or `Unknown` when flows are compiled out); either way
/// the result is exactly what `append_saved_flow_note` gives for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn carry_saved_flow_runs_its_lookups_off_the_worker() {
    let temp = thread_with_delivered_proposal("Friday Inbox Wrap-up");
    mark_proposal_saved_from_card(&temp, "flow-1");
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let thread = Some("thread-parent".to_string());
    let prompt = "Save the delivered proposal \"Friday Inbox Wrap-up\" as-is.".to_string();
    let carried = carry_saved_flow(&ctx, temp.path(), &thread, prompt.clone(), None).await;
    let expected = |presence| {
        append_saved_flow_note(
            prompt.clone(),
            "thread-parent",
            "workflow-proposal:sub-propose",
            "Friday Inbox Wrap-up",
            "flow-1",
            presence,
        )
    };
    assert_ne!(carried, prompt, "the saved proposal must be found");
    assert!(
        carried == expected(SavedFlowPresence::Missing)
            || carried == expected(SavedFlowPresence::Unknown),
        "{carried}"
    );
}

#[test]
fn carry_saved_flow_finds_the_older_saved_proposal_under_a_newer_unsaved_one() {
    // The user saved "Friday Inbox Wrap-up" from its card, then asked for a
    // second proposal that is still unsaved. Asking to save the first must
    // still carry its note; asking to save the second must not borrow it.
    let temp = thread_with_delivered_proposal("Friday Inbox Wrap-up");
    mark_proposal_saved_from_card(&temp, "flow-1");
    attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-propose-2",
        "workflow_builder",
        &[TranscriptMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Something Else","graph":{"nodes":[],"edges":[]}}"#,
        )],
        "proposal".to_string(),
    );
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let thread = Some("thread-parent".to_string());
    let save_first = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save the delivered proposal \"Friday Inbox Wrap-up\" as-is.".to_string(),
        None,
    );
    assert!(save_first.contains("flow-1"));
    assert!(save_first.contains("already saved"));
    let save_second = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save the delivered proposal \"Something Else\" as-is.".to_string(),
        None,
    );
    assert_eq!(
        save_second,
        "Save the delivered proposal \"Something Else\" as-is."
    );
}

#[test]
fn carry_saved_flow_matches_by_draft_id_not_name() {
    // An older saved "Daily" (draft-old, flow-1) and a newer unsaved "Daily"
    // (draft-new): a hand-off naming only "Daily" is about the newer one, so
    // it must not borrow the older save; a hand-off carrying a draft id
    // matches that proposal exactly.
    let temp = thread_with_delivered_proposal_extra("Daily", r#","draft_id":"draft-old""#);
    mark_proposal_saved_from_card(&temp, "flow-1");
    attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-propose-2",
        "workflow_builder",
        &[TranscriptMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Daily","graph":{"nodes":[],"edges":[]},"draft_id":"draft-new"}"#,
        )],
        "proposal".to_string(),
    );
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let thread = Some("thread-parent".to_string());
    let by_name = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save the delivered proposal \"Daily\" as-is.".to_string(),
        None,
    );
    assert_eq!(by_name, "Save the delivered proposal \"Daily\" as-is.");
    let by_new_draft = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save draft-new as-is.".to_string(),
        None,
    );
    assert_eq!(by_new_draft, "Save draft-new as-is.");
    let by_old_draft = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save draft-old as-is.".to_string(),
        None,
    );
    assert!(by_old_draft.contains("flow-1"));
    assert!(by_old_draft.contains("already saved"));
}

#[test]
fn carry_saved_flow_prefers_draft_id_over_a_shared_flow_id() {
    let temp = thread_with_delivered_proposal_extra(
        "Daily",
        r#", "draft_id":"draft-old", "flow_id":"flow-shared""#,
    );
    attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-propose-2",
        "workflow_builder",
        &[TranscriptMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Daily","graph":{"nodes":[]},"draft_id":"draft-new","flow_id":"flow-shared"}"#,
        )],
        "proposal".to_string(),
    );
    conversations::update_message(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path().to_path_buf(),
        "thread-parent",
        "workflow-proposal:sub-propose-2",
        conversations::ConversationMessagePatch {
            merge_extra_metadata: Some(json!({ "saved_flow_id": "flow-shared" })),
            ..Default::default()
        },
    )
    .expect("mark newer proposal saved");

    let prompt = "Save draft-old from flow-shared as-is.";
    let result = carry_saved_flow_matched(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        &Some("thread-parent".to_string()),
        prompt.to_string(),
        None,
    );
    assert_eq!(result, prompt);
}

#[test]
fn nested_subworkflow_id_does_not_select_the_saved_child_proposal() {
    let saved = ThreadWorkflowProposal {
        message_id: "saved-child".into(),
        name: "Child workflow".into(),
        draft_id: None,
        flow_id: Some("flow-child".into()),
        saved_flow_id: Some("flow-child".into()),
    };
    let hand_off = r#"Build a new proposal from this graph: {"nodes":[{"type":"sub_workflow","config":{"workflow_id":"flow-child"}}]}"#;
    assert!(hand_off_proposal(&[saved], hand_off).is_none());
}

#[test]
fn nested_subworkflow_name_does_not_select_the_saved_child_proposal() {
    let parent = ThreadWorkflowProposal {
        message_id: "parent".into(),
        name: "Morning digest".into(),
        draft_id: None,
        flow_id: None,
        saved_flow_id: None,
    };
    let child = ThreadWorkflowProposal {
        message_id: "saved-child".into(),
        name: "Fetch headlines and summarize".into(),
        draft_id: None,
        flow_id: Some("flow-child".into()),
        saved_flow_id: Some("flow-child".into()),
    };
    // The child's name is longer than the parent's, so a name match anywhere
    // in the text would pick the saved child over the unsaved parent.
    let hand_off = r#"Save "Morning digest" with this graph: {"nodes":[{"id":"n1","type":"sub_workflow","name":"Fetch headlines and summarize","config":{"workflow_id":"flow-child"}}]}"#;
    let proposals = [parent, child];
    let picked = hand_off_proposal(&proposals, hand_off).expect("the parent proposal");
    assert_eq!(picked.message_id, "parent");
}

#[test]
fn a_sub_workflow_object_nested_under_a_key_is_still_excluded() {
    let parent = ThreadWorkflowProposal {
        message_id: "parent".into(),
        name: "Morning digest".into(),
        draft_id: None,
        flow_id: None,
        saved_flow_id: None,
    };
    let child = ThreadWorkflowProposal {
        message_id: "saved-child".into(),
        name: "Fetch headlines and summarize".into(),
        draft_id: None,
        flow_id: Some("flow-child".into()),
        saved_flow_id: Some("flow-child".into()),
    };
    // The sub_workflow object is a property value (`"node": {…}`), and its
    // `workflow_id` sits in another property value (`"config": {…}`): the
    // first key inside each object must still be read as a key.
    // No quoted prose before the graph, so the scanner meets `"node": {`
    // with nothing but JSON before it.
    let hand_off = r#"Save the Morning digest proposal with {"node":{"type":"sub_workflow","name":"Fetch headlines and summarize","config":{"workflow_id":"flow-child"}}}"#;
    let proposals = [parent, child];
    let picked = hand_off_proposal(&proposals, hand_off).expect("the parent proposal");
    assert_eq!(picked.message_id, "parent");
}

#[test]
fn a_name_outside_the_graph_still_selects_its_proposal() {
    let saved = ThreadWorkflowProposal {
        message_id: "saved".into(),
        name: "Fetch headlines and summarize".into(),
        draft_id: None,
        flow_id: Some("flow-1".into()),
        saved_flow_id: Some("flow-1".into()),
    };
    // The name appears only as an ordinary (non-sub_workflow) node's name.
    let hand_off = r#"Save this graph: {"nodes":[{"id":"n1","type":"agent","name":"Fetch headlines and summarize"}]}"#;
    let proposals = [saved];
    let picked = hand_off_proposal(&proposals, hand_off).expect("the named proposal");
    assert_eq!(picked.message_id, "saved");
}

#[test]
fn carry_saved_flow_skips_ambiguous_same_name_proposals() {
    let temp = thread_with_delivered_proposal_extra("Daily", r#", "draft_id":"draft-old""#);
    mark_proposal_saved_from_card(&temp, "flow-old");
    attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-propose-2",
        "workflow_builder",
        &[TranscriptMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Daily","graph":{"nodes":[]},"draft_id":"draft-new"}"#,
        )],
        "proposal".to_string(),
    );
    conversations::update_message(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path().to_path_buf(),
        "thread-parent",
        "workflow-proposal:sub-propose-2",
        conversations::ConversationMessagePatch {
            merge_extra_metadata: Some(json!({ "saved_flow_id": "flow-new" })),
            ..Default::default()
        },
    )
    .expect("mark second proposal saved");

    let prompt = "Save the delivered proposal \"Daily\" as-is.";
    let result = carry_saved_flow_matched(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        &Some("thread-parent".to_string()),
        prompt.to_string(),
        None,
    );
    assert_eq!(result, prompt);
}

#[test]
fn carry_saved_flow_does_not_match_a_name_substring() {
    // An older saved "Daily" and a newer unsaved "Daily Report": asking to
    // save "Daily Report" must not borrow "Daily"'s flow id; asking to save
    // "Daily" still carries it.
    let temp = thread_with_delivered_proposal("Daily");
    mark_proposal_saved_from_card(&temp, "flow-1");
    attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-propose-2",
        "workflow_builder",
        &[TranscriptMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Daily Report","graph":{"nodes":[],"edges":[]}}"#,
        )],
        "proposal".to_string(),
    );
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let thread = Some("thread-parent".to_string());
    let save_report = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save the delivered proposal \"Daily Report\" as-is.".to_string(),
        None,
    );
    assert_eq!(
        save_report,
        "Save the delivered proposal \"Daily Report\" as-is."
    );
    let save_daily = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save the delivered proposal \"Daily\" as-is.".to_string(),
        None,
    );
    assert!(save_daily.contains("flow-1"));
    assert!(save_daily.contains("already saved"));
}

#[test]
fn a_deleted_flow_drops_the_note_and_any_other_lookup_error_keeps_it() {
    assert!(flow_get_means_missing("flow 'flow-1' not found"));
    assert!(!flow_get_means_missing("unknown method: flows.get"));
    assert!(!flow_get_means_missing(
        "flow 'flow-1' not found; catalog unavailable"
    ));
    assert!(!flow_get_means_missing("Config loading timed out"));
    assert_eq!(
        presence_from_flow_get(&Err("flow 'flow-1' not found".to_string())),
        SavedFlowPresence::Missing
    );
    assert_eq!(
        presence_from_flow_get(&Err("unknown method: flows.get".to_string())),
        SavedFlowPresence::Unknown
    );
    assert_eq!(
        presence_from_flow_get(&Ok(serde_json::json!({ "id": "flow-1" }))),
        SavedFlowPresence::Present
    );

    let prompt = "Save the delivered proposal \"Daily\" as-is.".to_string();
    let missing = append_saved_flow_note(
        prompt.clone(),
        "thread-parent",
        "workflow-proposal:sub-propose",
        "Daily",
        "flow-1",
        SavedFlowPresence::Missing,
    );
    assert!(missing.starts_with(&prompt));
    assert!(missing.contains("Ignore any earlier note"));
    assert!(missing.contains("may save the proposal again"));
    let unknown = append_saved_flow_note(
        prompt.clone(),
        "thread-parent",
        "workflow-proposal:sub-propose",
        "Daily",
        "flow-1",
        SavedFlowPresence::Unknown,
    );
    assert!(unknown.contains("flow-1"));
    assert!(unknown.contains("Do not call `create_workflow`"));
}

#[test]
fn carry_saved_flow_matches_a_name_only_in_context() {
    // The proposal name may live only in the hand-off context, not the
    // prompt: the lookup must search both.
    let temp = thread_with_delivered_proposal("Friday Inbox Wrap-up");
    mark_proposal_saved_from_card(&temp, "flow-1");
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let thread = Some("thread-parent".to_string());
    let save_it = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save the delivered proposal as-is.".to_string(),
        Some("The proposal is \"Friday Inbox Wrap-up\"."),
    );
    assert!(save_it.contains("flow-1"));
    assert!(save_it.contains("already saved"));
    let no_match = carry_saved_flow_matched(
        &ctx,
        temp.path(),
        &thread,
        "Save the delivered proposal as-is.".to_string(),
        Some("Nothing relevant here."),
    );
    assert_eq!(no_match, "Save the delivered proposal as-is.");
}
