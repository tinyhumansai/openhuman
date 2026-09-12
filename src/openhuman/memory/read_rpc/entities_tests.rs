//! Sibling tests for the entity read-RPCs.
//!
//! These handlers were covered only by `tests/raw_coverage/memory_threads_*`,
//! which drove them through an in-process engine alongside forty other
//! subjects. Those targets went with the engine (openhuman#6161) and took the
//! coverage with them — which was wrong, because what is worth pinning here was
//! never the engine's behaviour. It is the **host policy this module layers on
//! top of the contract**, and there are three separable pieces of it.
//!
//! The first is what happens when the bound driver does not serve the
//! `Entities` family at all. These are reads, so an honest empty answer is
//! right — the opposite of the destructive handlers next door, where
//! `admin_tests` pins that a delete a driver cannot perform must be an error
//! rather than a `rows_deleted: 0` success.
//!
//! The second is the caps. `top_entities_rpc` clamps the caller's `limit` and
//! `chunks_for_entity_rpc` imposes `MAX_LIST_LIMIT` where the SQL it replaced
//! had no bound at all.
//!
//! The third is the one that actually needed writing down, and it is why this
//! file exists rather than a note in the PR: `top_entities_rpc` deliberately
//! **degrades `MemoryError::Invalid` to an empty list**, because the contract
//! validates entity kinds and the SQL it replaced did not. The handler's own
//! comment calls that "the one behaviour delta this migration was warned
//! about", and it carries two narrowing conditions that are easy to lose in a
//! later edit — only `Invalid`, and only when a `kind` was supplied. Nothing
//! asserted either until now.

use super::{chunks_for_entity_rpc, delete_chunk_rpc, top_entities_rpc};
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::binding;
use std::sync::Arc;
use tempfile::TempDir;
use tinymemory_api::null::NullMemoryProvider;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.config_path = tmp.path().join("config.toml");
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    (tmp, cfg)
}

/// The placeholder driver: mandatory three families, `None` from every optional
/// accessor — so `as_entities()` is absent, which is the state these first
/// tests are about.
fn install_null_driver(cfg: &Config) {
    binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::new(NullMemoryProvider::new()) as Arc<dyn MemoryProvider>,
    );
}

/// A driver that serves `Entities` and answers `top_entities` the way the
/// contract specifies: `MemoryError::Invalid` for an unrecognised `kind`,
/// an empty vector otherwise.
///
/// Written here rather than reused from `tinymemory-conformance`, because that
/// crate's driver advertises `as_entities()` while leaving `top_entities` at
/// its trait default — which answers `Unsupported`. That is a real gap upstream
/// and worth filing, but it is not what these tests are about: the subject is
/// **this handler's** mapping of a driver's answer, so the instrument has to be
/// something that produces the answer being mapped.
#[derive(Debug, Default)]
struct EntityAwareDriver {
    /// Every `limit` the handler forwarded, so a clamp can be asserted by the
    /// value the driver *received* rather than by the shape of what came back.
    limits_seen: std::sync::Mutex<Vec<usize>>,
}

#[async_trait::async_trait]
impl tinymemory_api::provider::MemoryEntities for EntityAwareDriver {
    async fn entities(
        &self,
        _query: &str,
        _kind: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<tinymemory_api::provider::types::EntityHit>, tinymemory_api::error::MemoryError>
    {
        Ok(Vec::new())
    }

    async fn entity_edges(
        &self,
        _namespace: &str,
        _entity_id: &str,
        _limit: usize,
    ) -> Result<Vec<tinymemory_api::types::GraphRelationRecord>, tinymemory_api::error::MemoryError>
    {
        Ok(Vec::new())
    }

    async fn touch_entities(
        &self,
        _namespace: &str,
        _entity_ids: &[String],
    ) -> Result<(), tinymemory_api::error::MemoryError> {
        Ok(())
    }

    async fn top_entities(
        &self,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<
        Vec<tinymemory_api::provider::types::EntityOccurrence>,
        tinymemory_api::error::MemoryError,
    > {
        self.limits_seen.lock().expect("limits lock").push(limit);
        // The contract's request-side vocabulary. Only `person` is needed here;
        // what matters is that *some* kind is recognised and some other kind is
        // not, so the handler's two branches are both reachable.
        match kind {
            Some(k) if k != "person" => Err(tinymemory_api::error::MemoryError::Invalid(format!(
                "unknown entity kind: {k}"
            ))),
            _ => Ok(Vec::new()),
        }
    }
}

// The mandatory three. This driver is an instrument for one optional family,
// so they answer as the null driver does; only `top_entities` above is the
// subject.
#[async_trait::async_trait]
impl tinymemory_api::provider::MemoryCore for EntityAwareDriver {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: tinymemory_api::types::MemoryCategory,
        _session_id: Option<&str>,
        _taint: tinymemory_api::types::MemoryTaint,
    ) -> Result<(), tinymemory_api::error::MemoryError> {
        Ok(())
    }

    async fn get(
        &self,
        _namespace: &str,
        _key: &str,
    ) -> Result<Option<tinymemory_api::types::MemoryEntry>, tinymemory_api::error::MemoryError>
    {
        Ok(None)
    }

    async fn forget(
        &self,
        _namespace: &str,
        _key: &str,
    ) -> Result<bool, tinymemory_api::error::MemoryError> {
        Ok(false)
    }

    async fn namespaces(
        &self,
    ) -> Result<Vec<tinymemory_api::types::NamespaceSummary>, tinymemory_api::error::MemoryError>
    {
        Ok(Vec::new())
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&tinymemory_api::types::MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<tinymemory_api::types::MemoryEntry>, tinymemory_api::error::MemoryError> {
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl tinymemory_api::provider::MemoryRecall for EntityAwareDriver {
    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: &tinymemory_api::recall::OwnedRecallOpts,
        _scope: Option<&tinymemory_api::provider::SourceScope>,
    ) -> Result<Vec<tinymemory_api::types::MemoryEntry>, tinymemory_api::error::MemoryError> {
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl tinymemory_api::provider::MemoryPortability for EntityAwareDriver {
    async fn export_page(
        &self,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<tinymemory_api::provider::ExportPage, tinymemory_api::error::MemoryError> {
        Ok(tinymemory_api::provider::ExportPage::default())
    }

    async fn import_records(
        &self,
        _records: Vec<tinymemory_api::provider::ExportRecord>,
    ) -> Result<tinymemory_api::provider::ImportOutcome, tinymemory_api::error::MemoryError> {
        Ok(tinymemory_api::provider::ImportOutcome::default())
    }
}

#[async_trait::async_trait]
impl MemoryProvider for EntityAwareDriver {
    fn driver_id(&self) -> &str {
        "entity-aware-test-driver"
    }

    fn capabilities(&self) -> tinymemory_api::capabilities::Capabilities {
        tinymemory_api::capabilities::Capabilities::mandatory()
    }

    async fn health(&self) -> tinymemory_api::health::MemoryHealth {
        tinymemory_api::health::MemoryHealth::Ready
    }

    fn as_entities(&self) -> Option<&dyn tinymemory_api::provider::MemoryEntities> {
        Some(self)
    }
}

fn install_entity_driver(cfg: &Config) -> Arc<EntityAwareDriver> {
    let driver = Arc::new(EntityAwareDriver::default());
    binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Arc::clone(&driver) as Arc<dyn MemoryProvider>,
    );
    driver
}

// ── the family-absent reads ────────────────────────────────────────────────

#[tokio::test]
async fn top_entities_reports_empty_when_the_driver_has_no_entity_tier() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    let outcome = top_entities_rpc(&cfg, None, 10)
        .await
        .expect("a read must not fail because a family is unserved");
    assert!(
        outcome.value.is_empty(),
        "a driver with no entity tier has nothing indexed to rank"
    );
}

#[tokio::test]
async fn chunks_for_entity_reports_empty_when_the_driver_has_no_entity_tier() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    let outcome = chunks_for_entity_rpc(&cfg, "ent-1".to_string())
        .await
        .expect("a read must not fail because a family is unserved");
    assert!(outcome.value.is_empty());
}

/// Unlike the two reads above, this one *removes* things, so the
/// family-absent answer must not read as "there was nothing to remove".
#[tokio::test]
async fn delete_chunk_does_not_report_a_silent_success_without_the_family() {
    let (_tmp, cfg) = test_config();
    install_null_driver(&cfg);

    let outcome = delete_chunk_rpc(&cfg, "chunk-1".to_string()).await;
    assert!(
        outcome.is_err(),
        "a delete the driver cannot perform must be an error, not a `deleted: false` \
         success — a caller reading 'nothing was removed' concludes the content was \
         already gone. Got: {:?}",
        outcome.map(|ok| ok.value)
    );
}

// ── the caps ───────────────────────────────────────────────────────────────

/// `limit` is clamped at both ends before it reaches the driver.
///
/// Asserted on the value the driver was *handed*, which is the only thing that
/// can show a clamp: the earlier version of this test checked that a log line
/// existed and that the result was empty, and both are true whether or not the
/// handler clamps anything. `MAX_LIST_LIMIT` is 1,000 and the floor is 1.
#[tokio::test]
async fn top_entities_clamps_both_ends_of_the_limit_before_the_driver_sees_it() {
    let (_tmp, cfg) = test_config();
    let driver = install_entity_driver(&cfg);

    top_entities_rpc(&cfg, None, u32::MAX)
        .await
        .expect("top_entities");
    top_entities_rpc(&cfg, None, 0).await.expect("top_entities");

    let seen = driver.limits_seen.lock().expect("limits lock").clone();
    assert_eq!(
        seen,
        vec![super::MAX_LIST_LIMIT as usize, 1],
        "the ceiling must arrive as MAX_LIST_LIMIT and the floor as 1, not as \
         u32::MAX and 0"
    );
}

// ── the behaviour delta ────────────────────────────────────────────────────

/// An unrecognised `kind` comes back as an empty list, not an error.
///
/// This is the deliberate degradation the handler documents. The SQL this
/// replaced compared the string against a stored column, so an unknown kind
/// matched no rows; the contract member validates instead and answers
/// `Invalid`. Turning a quiet empty result into a user-visible error would be a
/// product change, so the handler maps it back.
#[tokio::test]
async fn an_unknown_kind_degrades_to_an_empty_list_rather_than_an_error() {
    let (_tmp, cfg) = test_config();
    let driver = install_entity_driver(&cfg);

    let outcome = top_entities_rpc(&cfg, Some("definitely-not-a-kind".to_string()), 10)
        .await
        .expect("an unknown kind must not surface as an error to the caller");
    assert!(
        outcome.value.is_empty(),
        "the pre-migration wire answered an empty list for an unknown kind"
    );
    // An empty list is also what an *unbound* driver produces, so the assertion
    // above cannot stand alone: it would pass if this test's driver were never
    // reached, and it did exactly that until the clamp test caught it.
    assert_eq!(
        driver.limits_seen.lock().expect("limits lock").len(),
        1,
        "the handler must have reached this driver — otherwise the empty list \
         above proves nothing about the map-back"
    );
}

/// The narrowing half: a *recognised* kind is forwarded and answered normally,
/// so the map-back above cannot be implemented as "swallow every filter".
#[tokio::test]
async fn a_recognised_kind_is_forwarded_rather_than_swallowed() {
    let (_tmp, cfg) = test_config();
    let driver = install_entity_driver(&cfg);

    let outcome = top_entities_rpc(&cfg, Some("person".to_string()), 10)
        .await
        .expect("a recognised kind is a normal query");
    assert!(outcome.value.is_empty(), "this driver indexes no entities");
    assert_eq!(
        driver.limits_seen.lock().expect("limits lock").len(),
        1,
        "the query must have reached the driver rather than being answered \
         by the map-back"
    );
}
