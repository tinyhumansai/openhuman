//! Domain RPC handlers for people. Adapter handlers in `schemas.rs`
//! parse params and delegate here.
//!
//! # These take the driver's people family, not a store
//!
//! They used to take `&PeopleStore` and reach the engine in-process. The store
//! lives behind the loaded module now, so each handler takes
//! `&dyn MemoryPeople` — the guarded family off the bound driver — and the
//! ranking, scoring and address-book work happens engine-side.
//!
//! What stays here is the **wire shape**: these payloads are a published RPC
//! surface (`people.*`) and the field names below are a compatibility surface,
//! so the JSON is assembled here rather than serialising contract types
//! directly. `schemas_tests` pins it.

use serde_json::{json, Value};

use crate::openhuman::memory::api::provider::{MemoryPeople, PersonHandle, PersonRecord};
use crate::rpc::RpcOutcome;

/// Render one person plus their score into the published `people.*` shape.
fn person_json(
    person: &PersonRecord,
    score: &crate::openhuman::memory::api::provider::PersonScore,
) -> Value {
    let handles: Vec<Value> = person
        .handles
        .iter()
        .map(|handle| {
            let (kind, value) = match handle {
                PersonHandle::IMessage(v) => ("imessage", v),
                PersonHandle::Email(v) => ("email", v),
                PersonHandle::DisplayName(v) => ("display_name", v),
            };
            json!({ "kind": kind, "value": value })
        })
        .collect();
    json!({
        "person_id": person.id,
        "display_name": person.display_name,
        "primary_email": person.primary_email,
        "primary_phone": person.primary_phone,
        "handles": handles,
        "score": score.score,
        "components": {
            "recency": score.recency,
            "frequency": score.frequency,
            "reciprocity": score.reciprocity,
            "depth": score.depth,
        },
        "interaction_count": score.interaction_count,
    })
}

/// List people ranked by composite score, highest first.
///
/// The ranking is the driver's — this no longer sorts. The engine holds the
/// interactions the score is computed from, so ranking host-side would mean
/// fetching every person's history across the bus to re-derive an order the
/// driver already produced.
pub async fn handle_list(
    people: &dyn MemoryPeople,
    limit: usize,
) -> Result<RpcOutcome<Value>, String> {
    let limit = limit.clamp(1, 500);
    let ranked = people
        .list_people(Some(limit))
        .await
        .map_err(|e| format!("list: {e}"))?;
    let people_json: Vec<Value> = ranked
        .iter()
        .map(|entry| person_json(&entry.person, &entry.score))
        .collect();
    Ok(RpcOutcome::new(json!({ "people": people_json }), vec![]))
}

/// Resolve a handle to a person id. Mints on first sight when
/// `create_if_missing` is true.
pub async fn handle_resolve(
    people: &dyn MemoryPeople,
    handle: PersonHandle,
    create_if_missing: bool,
) -> Result<RpcOutcome<Value>, String> {
    let resolved = people
        .resolve_handle(&handle, create_if_missing)
        .await
        .map_err(|e| format!("resolve: {e}"))?;
    Ok(RpcOutcome::new(
        json!({
            "person_id": resolved.as_ref().map(|r| r.id.clone()),
            "created": resolved.as_ref().is_some_and(|r| r.created),
        }),
        vec![],
    ))
}

/// Seed the people store from the system address book (CNContactStore on
/// macOS). Triggers the TCC Contacts permission prompt if not yet granted.
///
/// # `permission_denied` is always `false` now, and that is a real change
///
/// The contract deliberately reports a host without an address book — or
/// without permission to read it — as `seeded: 0` rather than as a distinct
/// error, because both mean the same thing to a caller and the alternative
/// leaks a platform detail into an engine-neutral contract. The field is kept
/// so the published shape does not change, but it can no longer become `true`.
/// Surfacing "grant Contacts access" needs a host-side permission probe, not a
/// memory-driver error.
pub async fn handle_refresh_address_book(
    people: &dyn MemoryPeople,
) -> Result<RpcOutcome<Value>, String> {
    let outcome = people
        .seed_from_address_book()
        .await
        .map_err(|e| format!("address_book: {e}"))?;
    log::debug!(
        "[people::rpc] refresh_address_book ok: seeded={} skipped={}",
        outcome.seeded,
        outcome.skipped
    );
    Ok(RpcOutcome::new(
        json!({
            "seeded": outcome.seeded,
            "skipped": outcome.skipped,
            "permission_denied": false,
        }),
        vec![],
    ))
}

/// Return the component-broken-down score for one person.
pub async fn handle_score(
    people: &dyn MemoryPeople,
    person_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let score = people
        .score_person(person_id)
        .await
        .map_err(|e| format!("score: {e}"))?
        .ok_or_else(|| format!("person not found: {person_id}"))?;
    Ok(RpcOutcome::new(
        json!({
            "person_id": person_id,
            "score": score.score,
            "components": {
                "recency": score.recency,
                "frequency": score.frequency,
                "reciprocity": score.reciprocity,
                "depth": score.depth,
            },
            "interaction_count": score.interaction_count,
        }),
        vec![],
    ))
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
