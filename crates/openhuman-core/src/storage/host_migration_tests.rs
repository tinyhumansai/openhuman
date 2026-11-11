//! Boot prepares each linked crate's declarations without a global registry.
//!
//! Record conversion is tested beside each typed owner importer. Seeding
//! arbitrary v1-named collections into a v2 backend did not exercise those
//! source readers or prove a migration.

use super::{boot::prepare_and_migrate, schema::collection_owners};
use crate::storage::{DocumentStoreExt, Scope};

#[tokio::test]
async fn boot_prepares_every_linked_owner_collection_in_shared_scopes() {
    let backend = crate::storage::test_support::memory_backend();
    let owners = collection_owners().unwrap();

    let report = prepare_and_migrate(&backend).await.unwrap();

    assert_eq!(report.scopes, 2);
    let expected = owners
        .iter()
        .map(|owner| owner.collections.len())
        .sum::<usize>();
    assert_eq!(report.collections, expected * report.scopes);
    for scope in [Scope::local(), Scope::new("cluster").unwrap()] {
        let storage = backend.for_scope(&scope).unwrap();
        for owner in &owners {
            for collection in &owner.collections {
                // A query fails if boot did not ensure the owner declaration.
                storage
                    .documents()
                    .query_all(&collection.name, &tinystoragedrivers_types::Query::all())
                    .await
                    .unwrap_or_else(|error| {
                        panic!(
                            "{} from {} in {scope}: {error}",
                            collection.name, owner.crate_name
                        )
                    });
            }
        }
    }
}
