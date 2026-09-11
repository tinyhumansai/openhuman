//! Handler tests for the goals domain.
//!
//! These used to run against a `tempfile::tempdir()` and the engine's
//! filesystem store. The handlers take the driver's family now, so the fixture
//! is a fake that implements exactly the two members `MemoryGoals` has —
//! which is also what makes the read-back contract visible: the fake stores
//! whatever it is given, so anything the handlers assert about the document
//! afterwards is something they read back rather than something they assumed.

use std::sync::Mutex;

use async_trait::async_trait;

use super::*;
use crate::openhuman::memory::api::error::MemoryError;

/// A `MemoryGoals` that keeps the document in memory.
///
/// Deliberately **not** cap-enforcing: the caps are the driver's and the
/// handlers must not depend on them, so a fixture that trimmed would hide a
/// handler that had quietly taken on that job.
#[derive(Default)]
struct FakeGoals {
    document: Mutex<GoalsDoc>,
    /// Every `set_goals` the handler made, so a test can assert the write count
    /// rather than only the end state.
    writes: Mutex<usize>,
}

#[async_trait]
impl MemoryGoals for FakeGoals {
    async fn goals(&self) -> Result<GoalsDoc, MemoryError> {
        Ok(self.document.lock().expect("goals fixture lock").clone())
    }

    async fn set_goals(&self, goals: GoalsDoc) -> Result<(), MemoryError> {
        *self.document.lock().expect("goals fixture lock") = goals;
        *self.writes.lock().expect("goals fixture lock") += 1;
        Ok(())
    }
}

/// A `MemoryGoals` whose writes always fail, for the refusal paths.
struct FailingGoals;

#[async_trait]
impl MemoryGoals for FailingGoals {
    async fn goals(&self) -> Result<GoalsDoc, MemoryError> {
        Ok(GoalsDoc::default())
    }

    async fn set_goals(&self, _goals: GoalsDoc) -> Result<(), MemoryError> {
        Err(MemoryError::Invalid("driver refused".to_string()))
    }
}

#[tokio::test]
async fn list_add_edit_delete_flow() {
    let goals = FakeGoals::default();

    // Starts empty.
    let listed = list(&goals).await.unwrap();
    assert!(listed.value.is_empty());

    // Add returns an id and the updated list.
    let added = add(&goals, "ship the desktop app").await.unwrap();
    let id = added.value.id.clone();
    assert_eq!(added.value.goals.items.len(), 1);

    // Edit by id.
    let edited = edit(&goals, &id, "ship the app to all platforms")
        .await
        .unwrap();
    assert_eq!(edited.value.items[0].text, "ship the app to all platforms");

    // Delete by id leaves the list empty.
    let deleted = delete(&goals, &id).await.unwrap();
    assert!(deleted.value.is_empty());

    // Unknown id is an error.
    assert!(edit(&goals, "nope", "x").await.is_err());
    assert!(delete(&goals, "nope").await.is_err());
}

/// The result a mutation reports is the document the *driver* holds, not the
/// one the handler assembled — which is the whole reason for the read-back.
#[tokio::test]
async fn a_mutation_reports_the_document_the_driver_kept() {
    let goals = FakeGoals::default();
    add(&goals, "first").await.unwrap();
    let second = add(&goals, "second").await.unwrap();

    assert_eq!(second.value.goals.items.len(), 2);
    let stored = goals.goals().await.unwrap();
    assert_eq!(
        second.value.goals, stored,
        "the reported list is read back, not reconstructed"
    );
}

/// Validation runs before the write, so a refused goal costs no `set_goals`
/// and cannot leave a partially-applied document behind.
#[tokio::test]
async fn invalid_text_is_refused_before_the_driver_is_asked() {
    let goals = FakeGoals::default();

    for bad in [
        "   ",
        "line one\n- [x] injected",
        "follow up with alice@example.com",
    ] {
        assert!(add(&goals, bad).await.is_err(), "expected {bad:?} refused");
    }

    assert_eq!(
        *goals.writes.lock().unwrap(),
        0,
        "a refused mutation must never reach set_goals"
    );
    assert!(goals.goals().await.unwrap().is_empty());
}

/// The refusal message is the specific one the agent acts on, not the driver's
/// catch-all.
#[tokio::test]
async fn refusal_names_the_specific_rule_that_was_broken() {
    let goals = FakeGoals::default();
    let err = add(&goals, "   ").await.unwrap_err();
    assert!(
        err.contains("goal text must not be empty"),
        "unexpected message: {err}"
    );
}

/// A driver that refuses the write is reported as a failure — never as a
/// success over the unchanged list.
#[tokio::test]
async fn a_driver_refusal_surfaces_rather_than_reading_back_a_stale_list() {
    let err = add(&FailingGoals, "ship the desktop app")
        .await
        .unwrap_err();
    assert!(err.starts_with("add: "), "unexpected message: {err}");
    assert!(err.contains("driver refused"), "unexpected message: {err}");
}
