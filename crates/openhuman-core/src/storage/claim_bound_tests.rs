//! A claim-bound handle declares its collections even on a backend whose
//! conditioned handles refuse to, as MongoDB's do (#8422), while a claim
//! that was lost still cannot write.

use super::*;

use serde_json::json;
use tinystoragedrivers_types::{
    CollectionSpec, Filter, Precondition, Query, StorageError, WatchOptions, WriteOptions,
};

use crate::storage::documents::Repo;
use crate::storage::import_state;
use crate::storage::test_support::refusing_conditioned_declarations;
use crate::storage::Scope;

const OWNER: &str = "claim_bound_test";
const ROWS: &str = "legacy_rows";

/// The refusing backend over a fresh SQLite database, and a repo on it.
fn refusing_repo() -> (tempfile::TempDir, Arc<dyn StorageBackend>, Repo) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("claims.sqlite").display());
    let sqlite = crate::storage::block_on(async move { crate::storage::open(&url).await }).unwrap();
    let backend = refusing_conditioned_declarations(sqlite);
    let repo = Repo::on(&backend, &Scope::local(), "claim-bound-test", Vec::new).unwrap();
    (dir, backend, repo)
}

/// Declares [`ROWS`] and writes one row named `id` through `docs`.
async fn declare_and_write(docs: Arc<dyn DocumentStore>, id: &'static str) -> Result<()> {
    docs.ensure_collection(&CollectionSpec::new(ROWS)).await?;
    docs.put(ROWS, id, json!({ "imported": true }), Precondition::Absent)
        .await
        .map(|_| ())
}

fn row(repo: &Repo, id: &'static str) -> Option<Versioned<Value>> {
    repo.run(move |docs| async move { docs.get(ROWS, id).await })
        .unwrap()
}

#[test]
fn the_test_backend_refuses_conditioned_declarations_like_mongodb() {
    let (_dir, backend, repo) = refusing_repo();
    let refused = import_state::with_claim(&repo, OWNER, "sanity", |condition| {
        let storage = backend.for_scope_conditioned(&Scope::local(), condition)?;
        let docs = Arc::clone(storage.documents());
        Ok(crate::storage::block_on(async move {
            docs.ensure_collection(&CollectionSpec::new(ROWS)).await
        }))
    })
    .unwrap()
    .expect("the claim is free");
    let error = refused.expect_err("a conditioned declaration is refused");
    assert!(
        error
            .to_string()
            .contains("cannot mutate collection declarations"),
        "{error}"
    );
}

#[test]
fn a_claimed_repo_declares_its_collections_and_writes() {
    let (_dir, _backend, repo) = refusing_repo();
    import_state::with_claim(&repo, OWNER, "repo", |condition| {
        repo.conditioned(condition)?
            .run(|docs| declare_and_write(docs, "via-repo"))
    })
    .unwrap()
    .expect("the claim is free");
    assert!(row(&repo, "via-repo").is_some());
}

#[test]
fn a_claim_bound_backend_declares_its_collections_and_writes() {
    let (_dir, backend, repo) = refusing_repo();
    import_state::with_claim(&repo, OWNER, "backend", |condition| {
        let bound = import_state::backend_with_condition(Arc::clone(&backend), condition.clone());
        let docs = Arc::clone(bound.for_scope(&Scope::local())?.documents());
        crate::storage::block_on(declare_and_write(docs, "via-backend"))?;
        let composed = bound.for_scope_conditioned(&Scope::local(), condition)?;
        let docs = Arc::clone(composed.documents());
        crate::storage::block_on(declare_and_write(docs, "via-composed"))?;
        Ok(())
    })
    .unwrap()
    .expect("the claim is free");
    assert!(row(&repo, "via-backend").is_some());
    assert!(row(&repo, "via-composed").is_some());
}

#[test]
fn a_lost_claim_may_declare_but_never_writes() {
    let (_dir, backend, repo) = refusing_repo();
    let stale = import_state::with_claim(&repo, OWNER, "lost", |condition| Ok(condition.clone()))
        .unwrap()
        .expect("the claim is free");
    // Released: its condition no longer holds.
    let docs = Arc::clone(
        claim_bound(backend.as_ref(), &Scope::local(), &stale)
            .unwrap()
            .documents(),
    );
    let (declared, written) = crate::storage::block_on(async move {
        let declared = docs.ensure_collection(&CollectionSpec::new(ROWS)).await;
        let written = docs
            .put(ROWS, "stale", json!({}), Precondition::Absent)
            .await
            .map(|_| ());
        Ok::<_, StorageError>((declared, written))
    })
    .unwrap();
    declared.expect("a lost claim may still declare a collection");
    assert!(written.is_err(), "a stale claim cannot write");
    assert!(row(&repo, "stale").is_none());
}

#[test]
fn every_other_document_operation_stays_under_the_claim() {
    let (_dir, backend, repo) = refusing_repo();
    import_state::with_claim(&repo, OWNER, "passthrough", |condition| {
        let storage = claim_bound(backend.as_ref(), &Scope::local(), condition)?;
        let docs = Arc::clone(storage.documents());
        assert!(format!("{docs:?}").contains("ClaimBoundDocuments"));
        let claimed = backend.for_scope_conditioned(&Scope::local(), condition)?;
        assert_eq!(docs.capabilities(), claimed.documents().capabilities());
        let claimed = Arc::clone(claimed.documents());
        crate::storage::block_on(async move {
            let spec = CollectionSpec::new(ROWS);
            docs.ensure_collection(&spec).await?;
            docs.validate_collection_spec(&spec).await?;
            docs.put(ROWS, "a", json!({ "n": 1 }), Precondition::Absent)
                .await?;
            docs.put_with(
                ROWS,
                "b",
                json!({ "n": 2 }),
                Precondition::Absent,
                WriteOptions::new(),
            )
            .await?;
            assert!(docs.get(ROWS, "a").await?.is_some());
            assert_eq!(docs.count(ROWS, &Filter::All).await?, 2);
            assert_eq!(docs.query(ROWS, &Query::all()).await?.items.len(), 2);
            assert!(docs.delete(ROWS, "a", Precondition::None).await?);
            let _ = docs
                .claim(ROWS, &Filter::All, &[], &json!({ "n": 3 }))
                .await;
            // The optional operations answer exactly as the claimed handle does.
            assert_eq!(
                docs.atomic_batch(Vec::new()).await.is_ok(),
                claimed.atomic_batch(Vec::new()).await.is_ok()
            );
            assert_eq!(
                docs.atomic_batch_checked(Vec::new(), Vec::new())
                    .await
                    .is_ok(),
                claimed
                    .atomic_batch_checked(Vec::new(), Vec::new())
                    .await
                    .is_ok()
            );
            assert_eq!(
                docs.search(ROWS, "n", 1).await.is_ok(),
                claimed.search(ROWS, "n", 1).await.is_ok()
            );
            assert_eq!(
                docs.collections("").await.is_ok(),
                claimed.collections("").await.is_ok()
            );
            let watch = WatchOptions::default();
            assert_eq!(
                docs.watch(ROWS, &Filter::All, &watch).await.is_ok(),
                claimed.watch(ROWS, &Filter::All, &watch).await.is_ok()
            );
            assert_eq!(docs.delete_where(ROWS, &Filter::All).await?, 1);
            docs.drop_collection(ROWS).await?;
            Ok::<_, StorageError>(())
        })?;
        Ok(())
    })
    .unwrap()
    .expect("the claim is free");
}
