//! Tests for the surrounding module.
//!
//! The task-local came home in #5560, so its normalisation and scoping rules
//! are this crate's to pin: an unset scope must read as `None` (the provider
//! then omits the field rather than sending an empty one), whitespace-only ids
//! must normalise to `None` rather than reaching the wire, and a nested scope
//! must shadow its parent for the duration and no longer.

use super::{current_thread_id, with_thread_id};

#[tokio::test]
async fn reads_the_id_set_by_the_enclosing_scope() {
    with_thread_id("abc123", async {
        assert_eq!(current_thread_id().as_deref(), Some("abc123"));
    })
    .await;
}

#[tokio::test]
async fn outside_any_scope_there_is_no_ambient_id() {
    assert_eq!(current_thread_id(), None);
}

#[tokio::test]
async fn blank_ids_normalise_to_none() {
    with_thread_id("   ", async {
        assert_eq!(current_thread_id(), None);
    })
    .await;
    with_thread_id("", async {
        assert_eq!(current_thread_id(), None);
    })
    .await;
}

#[tokio::test]
async fn surrounding_whitespace_is_trimmed() {
    with_thread_id("  t-42\n", async {
        assert_eq!(current_thread_id().as_deref(), Some("t-42"));
    })
    .await;
}

#[tokio::test]
async fn a_nested_scope_shadows_its_parent_and_then_restores_it() {
    with_thread_id("outer", async {
        assert_eq!(current_thread_id().as_deref(), Some("outer"));
        with_thread_id("inner", async {
            assert_eq!(current_thread_id().as_deref(), Some("inner"));
        })
        .await;
        assert_eq!(current_thread_id().as_deref(), Some("outer"));
    })
    .await;
}

/// A spawned task does not inherit the parent's task-local, which is why the
/// orchestrator's async sub-agent path re-enters `with_thread_id` explicitly
/// rather than relying on the scope it was spawned from.
#[tokio::test]
async fn a_spawned_task_does_not_inherit_the_scope() {
    with_thread_id("parent", async {
        let inherited = tokio::spawn(async { current_thread_id() })
            .await
            .expect("spawned task panicked");
        assert_eq!(inherited, None);
    })
    .await;
}
