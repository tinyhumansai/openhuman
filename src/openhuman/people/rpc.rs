//! Domain RPC handlers for people. Adapter handlers in `schemas.rs`
//! parse params and delegate here. Tests can call these functions
//! directly with a constructed `PeopleStore`.

use chrono::Utc;
use serde_json::{json, Value};

use crate::openhuman::people::address_book::{AddressBookError, SystemContactsSource};
use crate::openhuman::people::resolver::HandleResolver;
use crate::openhuman::people::scorer::score;
use crate::openhuman::people::store::PeopleStore;
use crate::openhuman::people::types::{Handle, PersonId};
use crate::rpc::RpcOutcome;

/// List people ranked by composite score, highest first.
pub async fn handle_list(store: &PeopleStore, limit: usize) -> Result<RpcOutcome<Value>, String> {
    let limit = limit.clamp(1, 500);
    let people = store.list().await.map_err(|e| format!("list: {e}"))?;
    let now = Utc::now();

    let mut ranked: Vec<(Value, f32)> = Vec::with_capacity(people.len());
    for p in people {
        let interactions = store
            .interactions_for(p.id)
            .await
            .map_err(|e| format!("interactions_for: {e}"))?;
        let s = score(&interactions, now);
        let handles: Vec<Value> = p
            .handles
            .iter()
            .map(|h| {
                let (kind, value) = h.as_key();
                json!({ "kind": kind, "value": value })
            })
            .collect();
        ranked.push((
            json!({
                "person_id": p.id.to_string(),
                "display_name": p.display_name,
                "primary_email": p.primary_email,
                "primary_phone": p.primary_phone,
                "handles": handles,
                "score": s.score,
                "components": {
                    "recency": s.recency,
                    "frequency": s.frequency,
                    "reciprocity": s.reciprocity,
                    "depth": s.depth,
                },
                "interaction_count": interactions.len(),
            }),
            s.score,
        ));
    }
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let people_json: Vec<Value> = ranked.into_iter().take(limit).map(|(v, _)| v).collect();
    Ok(RpcOutcome::new(json!({ "people": people_json }), vec![]))
}

/// Resolve a handle to a `PersonId`. Mints on first sight when
/// `create_if_missing` is true.
pub async fn handle_resolve(
    store: &PeopleStore,
    handle: Handle,
    create_if_missing: bool,
) -> Result<RpcOutcome<Value>, String> {
    let resolver = HandleResolver::new(store);
    let result = if create_if_missing {
        Some(resolver.resolve_or_create(&handle).await?)
    } else {
        resolver.resolve(&handle).await?
    };
    Ok(RpcOutcome::new(
        json!({
            "person_id": result.map(|p| p.to_string()),
            "created": create_if_missing && result.is_some(),
        }),
        vec![],
    ))
}

/// Seed the people store from the system address book (CNContactStore on
/// macOS). Triggers the TCC Contacts permission prompt if not yet granted.
///
/// Returns counts of seeded and skipped contacts, plus a `permission_denied`
/// flag so callers can surface an actionable message to the user.
pub async fn handle_refresh_address_book(store: &PeopleStore) -> Result<RpcOutcome<Value>, String> {
    let resolver = HandleResolver::new(store);
    let source = SystemContactsSource;
    match resolver.seed_from_address_book(&source).await {
        Ok((seeded, skipped)) => {
            tracing::debug!(
                "[people::rpc] refresh_address_book ok: seeded={seeded} skipped={skipped}"
            );
            Ok(RpcOutcome::new(
                json!({
                    "seeded": seeded,
                    "skipped": skipped,
                    "permission_denied": false,
                }),
                vec![],
            ))
        }
        Err(AddressBookError::PermissionDenied) => {
            tracing::warn!("[people::rpc] refresh_address_book: contacts permission denied");
            Ok(RpcOutcome::new(
                json!({
                    "seeded": 0,
                    "skipped": 0,
                    "permission_denied": true,
                }),
                vec![],
            ))
        }
        Err(AddressBookError::Other(e)) => Err(format!("address_book: {e}")),
    }
}

/// Return the component-broken-down score for one person.
pub async fn handle_score(
    store: &PeopleStore,
    person_id: PersonId,
) -> Result<RpcOutcome<Value>, String> {
    let interactions = store
        .interactions_for(person_id)
        .await
        .map_err(|e| format!("interactions_for: {e}"))?;
    let s = score(&interactions, Utc::now());
    Ok(RpcOutcome::new(
        json!({
            "person_id": person_id.to_string(),
            "score": s.score,
            "components": {
                "recency": s.recency,
                "frequency": s.frequency,
                "reciprocity": s.reciprocity,
                "depth": s.depth,
            },
            "interaction_count": interactions.len(),
        }),
        vec![],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::people::types::{Interaction, Person};
    use chrono::Duration;

    #[tokio::test]
    async fn list_orders_by_score_desc() {
        let store = PeopleStore::open_in_memory().unwrap();
        let now = Utc::now();

        // Person A: strong two-way conversation, recent.
        let a = PersonId::new();
        store
            .insert_person(
                &Person {
                    id: a,
                    display_name: Some("Alice".into()),
                    primary_email: Some("a@x.z".into()),
                    primary_phone: None,
                    handles: vec![],
                    created_at: now,
                    updated_at: now,
                },
                &[Handle::Email("a@x.z".into())],
            )
            .await
            .unwrap();
        for i in 0..10 {
            store
                .record_interaction(Interaction {
                    person_id: a,
                    ts: now - Duration::hours(i),
                    is_outbound: i % 2 == 0,
                    length: 300,
                })
                .await
                .unwrap();
        }

        // Person B: quiet, only one old outbound.
        let b = PersonId::new();
        store
            .insert_person(
                &Person {
                    id: b,
                    display_name: Some("Bob".into()),
                    primary_email: Some("b@x.z".into()),
                    primary_phone: None,
                    handles: vec![],
                    created_at: now,
                    updated_at: now,
                },
                &[Handle::Email("b@x.z".into())],
            )
            .await
            .unwrap();
        store
            .record_interaction(Interaction {
                person_id: b,
                ts: now - Duration::days(60),
                is_outbound: true,
                length: 20,
            })
            .await
            .unwrap();

        let outcome = handle_list(&store, 10).await.unwrap();
        let arr = outcome.value["people"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["display_name"], "Alice");
        assert_eq!(arr[1]["display_name"], "Bob");
        let alice_score = arr[0]["score"].as_f64().unwrap();
        let bob_score = arr[1]["score"].as_f64().unwrap();
        assert!(alice_score > bob_score);
    }

    #[tokio::test]
    async fn resolve_without_create_returns_null_for_unknown() {
        let store = PeopleStore::open_in_memory().unwrap();
        let outcome = handle_resolve(&store, Handle::Email("x@y.z".into()), false)
            .await
            .unwrap();
        assert!(outcome.value["person_id"].is_null());
    }
}
