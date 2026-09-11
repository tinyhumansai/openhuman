//! Raw integration coverage for the host→bus source-scope rendering that the
//! retrieval handlers rewired onto in #5854.
//!
//! #5854 moved `read_rpc/chunks.rs`'s recall and the `tree/retrieval/rpc.rs`
//! handlers off a direct engine call and onto
//! `binding.provider().as_retrieval()` with an **explicit**
//! `source_scope::as_bus_scope()` argument. The scope used to be a task-local
//! the engine read for itself; once memory is a separately compiled module
//! with its own statics, a task-local set on this side is invisible on the
//! other, so the value has to cross the bus.
//!
//! `as_bus_scope` is the join, and its contract has one asymmetry that is easy
//! to get backwards and impossible to see from a passing build:
//!
//! - **`None` means unrestricted** and must stay `None`.
//! - **`Some(SourceScope)` with an empty allow list denies every
//!   source-attributed item** (`SourceScope`'s own fail-closed rule).
//!
//! So collapsing "no restriction" onto `Some(SourceScope::default())` inverts
//! the policy: recall silently returns nothing instead of everything. Both
//! values are well-formed, both type-check, and the handlers report an empty
//! page as a legitimate answer — so the failure surfaces as "memory forgot
//! everything", with nothing in the logs.
//!
//! Nothing asserted this before: every occurrence of `as_bus_scope` in the tree
//! is a call site or a comment.

use std::collections::HashSet;

use openhuman_core::openhuman::memory::source_scope::{as_bus_scope, with_source_scope};

/// Outside any scope, and for an explicit `None`, the rendering stays `None`.
///
/// This is the inversion guard. `Some(empty)` here would be a scope that denies
/// everything, which is the exact opposite of what an unscoped turn means.
#[tokio::test]
async fn unrestricted_recall_renders_as_no_scope_not_an_empty_allowlist() {
    assert!(
        as_bus_scope().is_none(),
        "outside any scope the bus argument must be None (unrestricted); \
         Some(empty) is SourceScope's deny-all and would blank recall"
    );

    with_source_scope(None, async {
        assert!(
            as_bus_scope().is_none(),
            "an explicit None allowlist must render as None, not as an empty \
             SourceScope"
        );
    })
    .await;
}

/// A profile that selected no sources is a real restriction, and must NOT be
/// flattened into the unrestricted `None`.
///
/// The mirror of the test above: these two inputs are the ones that must not be
/// confused, in either direction.
#[tokio::test]
async fn an_empty_allowlist_renders_as_a_scope_that_denies_every_source() {
    with_source_scope(Some(vec![]), async {
        let scope = as_bus_scope().expect(
            "an empty allowlist is a restriction the driver must be told about, \
             not the absence of one",
        );
        assert!(
            scope.is_empty(),
            "an empty allowlist must cross the bus as an empty SourceScope"
        );
        assert!(
            !scope.allows_source_id("mem_src:anything:item-1"),
            "an empty scope denies all source-attributed content"
        );
    })
    .await;
}

/// The allowlist reaches the driver intact, and matches by the bus's own rule.
///
/// `SourceScope::allows_source_id` accepts an outright id match or the
/// `mem_src:<allowed>:<item>` composite the reader-based sources emit. Asserting
/// through that predicate — rather than comparing the `allow` vector — is what
/// ties the host's rendering to the matching the driver will actually perform.
#[tokio::test]
async fn the_allowlist_crosses_the_bus_and_matches_by_the_drivers_rule() {
    let allowed = vec!["gmail:work".to_string(), "src-abc".to_string()];

    with_source_scope(Some(allowed.clone()), async {
        let scope = as_bus_scope().expect("a non-empty allowlist must render as Some");

        let carried: HashSet<&str> = scope.allow.iter().map(String::as_str).collect();
        assert_eq!(
            carried,
            allowed.iter().map(String::as_str).collect::<HashSet<_>>(),
            "every allowlisted source must reach the driver, and no others"
        );

        assert!(
            scope.allows_source_id("src-abc"),
            "an outright id match is in scope"
        );
        assert!(
            scope.allows_source_id("mem_src:src-abc:item-1"),
            "the mem_src composite the reader-based sources emit is in scope"
        );
        assert!(
            !scope.allows_source_id("src-xyz"),
            "a source outside the allowlist stays out"
        );
    })
    .await;
}
