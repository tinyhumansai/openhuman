use super::*;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::{
    AddressBookSeedOutcome, PersonInteraction, PersonRecord, PersonScore, RankedPerson,
    ResolvedPerson,
};
use async_trait::async_trait;

/// A people family that answers with canned values.
///
/// These tests cover what stayed **host-side** after the module port: the
/// published `people.*` JSON shape, and that the driver's ordering is
/// passed through rather than re-derived. Ranking and scoring themselves
/// moved into the engine and are tested there — asserting them here would
/// only re-test the fake.
struct FakePeople {
    ranked: Vec<RankedPerson>,
    resolved: Option<ResolvedPerson>,
}

fn person(id: &str, name: &str) -> PersonRecord {
    PersonRecord {
        id: id.to_string(),
        display_name: Some(name.to_string()),
        primary_email: Some(format!("{name}@x.z").to_lowercase()),
        primary_phone: None,
        handles: vec![PersonHandle::Email(format!("{name}@x.z").to_lowercase())],
        created_at: "2026-01-01T00:00:00+00:00".into(),
        updated_at: "2026-01-01T00:00:00+00:00".into(),
    }
}

fn scored(score: f32, interactions: usize) -> PersonScore {
    PersonScore {
        recency: score,
        frequency: score,
        reciprocity: score,
        depth: score,
        score,
        interaction_count: interactions,
    }
}

#[async_trait]
impl MemoryPeople for FakePeople {
    async fn list_people(&self, _limit: Option<usize>) -> Result<Vec<RankedPerson>, MemoryError> {
        Ok(self.ranked.clone())
    }
    async fn get_person(&self, _id: &str) -> Result<Option<PersonRecord>, MemoryError> {
        Ok(None)
    }
    async fn resolve_handle(
        &self,
        _handle: &PersonHandle,
        _create_if_missing: bool,
    ) -> Result<Option<ResolvedPerson>, MemoryError> {
        Ok(self.resolved.clone())
    }
    async fn add_handle_alias(&self, _id: &str, _handle: &PersonHandle) -> Result<(), MemoryError> {
        Ok(())
    }
    async fn score_person(&self, _id: &str) -> Result<Option<PersonScore>, MemoryError> {
        Ok(Some(scored(0.5, 7)))
    }
    async fn record_interaction(
        &self,
        _interaction: &PersonInteraction,
    ) -> Result<(), MemoryError> {
        Ok(())
    }
    async fn seed_from_address_book(&self) -> Result<AddressBookSeedOutcome, MemoryError> {
        Ok(AddressBookSeedOutcome {
            seeded: 3,
            skipped: 1,
        })
    }
}

#[tokio::test]
async fn list_preserves_the_drivers_order_and_published_shape() {
    let people = FakePeople {
        ranked: vec![
            RankedPerson {
                person: person("id-a", "Alice"),
                score: scored(0.9, 10),
            },
            RankedPerson {
                person: person("id-b", "Bob"),
                score: scored(0.1, 1),
            },
        ],
        resolved: None,
    };
    let outcome = handle_list(&people, 10).await.unwrap();
    let arr = outcome.value["people"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // Order is the driver's, not re-sorted here.
    assert_eq!(arr[0]["display_name"], "Alice");
    assert_eq!(arr[1]["display_name"], "Bob");
    // The published field set, which is a compatibility surface.
    assert_eq!(arr[0]["person_id"], "id-a");
    assert_eq!(arr[0]["interaction_count"], 10);
    // Compared with tolerance: the contract's score components are `f32`
    // and JSON numbers are `f64`, so 0.9f32 widens to 0.8999999761581421.
    // An exact assertion here would pin a widening artefact, not behaviour.
    let recency = arr[0]["components"]["recency"].as_f64().unwrap();
    assert!(
        (recency - 0.9).abs() < 1e-6,
        "recency component should round-trip: {recency}"
    );
    assert_eq!(arr[0]["handles"][0]["kind"], "email");
}

#[tokio::test]
async fn list_does_not_re_sort_what_the_driver_returned() {
    // Deliberately out of score order: the driver is the ranking authority,
    // so a host-side sort would silently override it.
    let people = FakePeople {
        ranked: vec![
            RankedPerson {
                person: person("id-low", "Low"),
                score: scored(0.1, 1),
            },
            RankedPerson {
                person: person("id-high", "High"),
                score: scored(0.9, 9),
            },
        ],
        resolved: None,
    };
    let outcome = handle_list(&people, 10).await.unwrap();
    let arr = outcome.value["people"].as_array().unwrap();
    assert_eq!(arr[0]["display_name"], "Low");
    assert_eq!(arr[1]["display_name"], "High");
}

#[tokio::test]
async fn resolve_without_create_returns_null_for_unknown() {
    let people = FakePeople {
        ranked: vec![],
        resolved: None,
    };
    let outcome = handle_resolve(&people, PersonHandle::Email("x@y.z".into()), false)
        .await
        .unwrap();
    assert!(outcome.value["person_id"].is_null());
    assert_eq!(outcome.value["created"], false);
}

#[tokio::test]
async fn resolve_reports_whether_the_person_was_minted() {
    let people = FakePeople {
        ranked: vec![],
        resolved: Some(ResolvedPerson {
            id: "id-new".into(),
            created: true,
        }),
    };
    let outcome = handle_resolve(&people, PersonHandle::Email("x@y.z".into()), true)
        .await
        .unwrap();
    assert_eq!(outcome.value["person_id"], "id-new");
    assert_eq!(outcome.value["created"], true);
}

#[tokio::test]
async fn score_carries_the_interaction_count_alongside_the_components() {
    let people = FakePeople {
        ranked: vec![],
        resolved: None,
    };
    let outcome = handle_score(&people, "id-a").await.unwrap();
    assert_eq!(outcome.value["person_id"], "id-a");
    assert_eq!(outcome.value["interaction_count"], 7);
    // 0.5 is exactly representable in both f32 and f64, so this one can be
    // compared directly.
    assert_eq!(outcome.value["components"]["depth"], 0.5);
}

/// `permission_denied` is now always `false` — see the handler docs.
#[tokio::test]
async fn refresh_address_book_reports_counts_and_never_a_permission_denial() {
    let people = FakePeople {
        ranked: vec![],
        resolved: None,
    };
    let outcome = handle_refresh_address_book(&people).await.unwrap();
    assert_eq!(outcome.value["seeded"], 3);
    assert_eq!(outcome.value["skipped"], 1);
    assert_eq!(outcome.value["permission_denied"], false);
}
