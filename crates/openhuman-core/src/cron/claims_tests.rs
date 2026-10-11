use super::*;
use crate::cron::{Schedule, SessionTarget};
use crate::storage::{MemoryStorage, Scope, StorageBackend};
use chrono::Duration;
use std::sync::Arc;
use tinyflows_drivers::schedule::CronDocuments;

async fn job(docs: &Arc<dyn DocumentStore>) -> CronJob {
    CronDocuments::new(Arc::clone(docs))
        .add_agent_job(
            None,
            Schedule::Every {
                every_ms: 3_600_000,
            },
            "ping",
            SessionTarget::Isolated,
            None,
            None,
            false,
        )
        .await
        .unwrap()
}

fn docs() -> Arc<dyn DocumentStore> {
    let scoped = MemoryStorage::new()
        .for_scope(&Scope::new("claims").unwrap())
        .unwrap();
    Arc::clone(scoped.documents())
}

#[tokio::test]
async fn competing_holders_claim_an_occurrence_once() {
    let docs = docs();
    let job = job(&docs).await;
    let now = Utc::now();
    let (a, b) = tokio::join!(
        claim_occurrence(docs.as_ref(), &job, "node-a", now),
        claim_occurrence(docs.as_ref(), &job, "node-b", now),
    );
    assert_eq!(
        [a.unwrap(), b.unwrap()].iter().filter(|won| **won).count(),
        1
    );
    let again = claim_occurrence(docs.as_ref(), &job, "node-a", now).await;
    assert!(
        !again.unwrap(),
        "a holder cannot claim the same occurrence twice"
    );
}

#[tokio::test]
async fn a_lapsed_claim_is_retaken_by_one_holder() {
    let docs = docs();
    let job = job(&docs).await;
    let now = Utc::now();
    assert!(claim_occurrence(docs.as_ref(), &job, "dead", now)
        .await
        .unwrap());
    let later = now + Duration::milliseconds(CLAIM_LEASE_MS + 1);
    let (a, b) = tokio::join!(
        claim_occurrence(docs.as_ref(), &job, "node-a", later),
        claim_occurrence(docs.as_ref(), &job, "node-b", later),
    );
    assert_eq!(
        [a.unwrap(), b.unwrap()].iter().filter(|won| **won).count(),
        1
    );
}

#[tokio::test]
async fn each_occurrence_has_its_own_claim_and_old_claims_are_pruned() {
    let docs = docs();
    let mut job = job(&docs).await;
    let now = Utc::now();
    assert!(claim_occurrence(docs.as_ref(), &job, "node-a", now)
        .await
        .unwrap());
    job.next_run += Duration::hours(1);
    assert!(claim_occurrence(docs.as_ref(), &job, "node-a", now)
        .await
        .unwrap());
    assert_eq!(prune(docs.as_ref(), now).await.unwrap(), 0);
    assert_eq!(
        prune(docs.as_ref(), now + Duration::days(2)).await.unwrap(),
        2
    );
}
