use super::*;

static SLOT: HookSlot<u32> = HookSlot::new("slots_tests::numbers");

#[test]
fn an_empty_slot_has_no_entries() {
    let slot: HookSlot<u32> = HookSlot::new("slots_tests::empty");
    assert!(slot.is_empty());
    assert!(slot.entries().is_empty());
    assert_eq!(slot.get("missing"), None);
    assert_eq!(slot.first(), None);
}

#[test]
fn entries_keep_registration_order_and_re_registering_keeps_position() {
    SLOT.clear();
    SLOT.register("b", 2);
    SLOT.register("a", 1);
    SLOT.register("c", 3);
    SLOT.register("b", 20);
    let entries = SLOT.entries();
    assert_eq!(
        entries,
        vec![("b".into(), 20), ("a".into(), 1), ("c".into(), 3)]
    );
    assert_eq!(SLOT.first(), Some(20));
    assert_eq!(SLOT.get("a"), Some(1));
    SLOT.clear();
}

#[test]
fn unregister_removes_only_the_named_entry() {
    SLOT.clear();
    SLOT.register("a", 1);
    SLOT.register("b", 2);
    assert!(SLOT.unregister("a"));
    assert!(!SLOT.unregister("a"));
    assert_eq!(SLOT.entries(), vec![("b".to_string(), 2)]);
    SLOT.clear();
}

#[test]
fn context_registrations_are_isolated_until_an_extensions_set_is_bound() {
    SLOT.clear();
    let extensions = Extensions::new();
    SLOT.register_in(&extensions, "ctx", 7);
    // Nothing is bound to an ambient context here, so the process view is empty.
    assert!(SLOT.entries().is_empty());
    let table = extensions.existing::<u32>("slots_tests::numbers").unwrap();
    assert_eq!(table.get("ctx"), Some(7));
}

#[test]
fn explicit_process_lookup_never_falls_through_to_ambient_context() {
    use crate::core::runtime::{CoreContext, DomainSet};

    let slot: HookSlot<u32> = HookSlot::new("slots_tests::explicit_lookup");
    slot.clear();
    slot.register("process", 1);
    let context = CoreContext::for_test(DomainSet::full(), None);
    slot.register_in(context.extensions(), "ambient", 2);

    let actual = CoreContext::sync_scope(context.clone(), || {
        (
            slot.get_for(None, "process"),
            slot.get_for(None, "ambient"),
            slot.get_for(Some(context.extensions()), "ambient"),
        )
    });
    assert_eq!(actual, (Some(1), None, Some(2)));
    slot.clear();
}

#[test]
fn merging_extensions_shares_tables_without_overwriting_existing_ones() {
    let first = Extensions::new();
    let second = Extensions::new();
    SLOT.register_in(&first, "one", 1);
    SLOT.register_in(&second, "two", 2);
    let target = Extensions::new();
    target.merge_from(&first);
    target.merge_from(&second);
    assert_eq!(target.len(), 1, "the first table for a slot name wins");
    let table = target.existing::<u32>("slots_tests::numbers").unwrap();
    assert_eq!(table.get("one"), Some(1));
    assert_eq!(table.get("two"), None);
}

#[test]
fn a_slot_name_reused_with_another_type_yields_a_detached_table() {
    let extensions = Extensions::new();
    SLOT.register_in(&extensions, "n", 1);
    let other: HookSlot<String> = HookSlot::new("slots_tests::numbers");
    other.register_in(&extensions, "s", "x".to_string());
    let table = extensions.existing::<u32>("slots_tests::numbers").unwrap();
    assert_eq!(table.get("n"), Some(1));
    assert_eq!(table.get("s"), None);
}

/// A context registration must win `first()` over a process-wide one, as it
/// does in `get`, even when the ids differ (a process entry is registered first
/// in `entries()` order).
#[test]
fn a_context_entry_shadows_a_process_entry_in_first() {
    use crate::core::runtime::{CoreContext, DomainSet};
    let slot: HookSlot<u32> = HookSlot::new("slots_tests::shadow");
    slot.register("process", 1);
    let ctx = CoreContext::for_test(DomainSet::full(), None);
    slot.register_in(ctx.extensions(), "context", 2);

    assert_eq!(
        slot.first(),
        Some(1),
        "outside the context only the process entry exists"
    );
    let inside = CoreContext::sync_scope(Arc::clone(&ctx), || {
        (slot.first(), slot.entries_context_first())
    });
    assert_eq!(inside.0, Some(2));
    assert_eq!(
        inside.1,
        vec![("context".to_string(), 2), ("process".to_string(), 1)]
    );
    slot.clear();
}

#[test]
fn a_context_entry_replacing_a_process_id_is_not_listed_twice() {
    use crate::core::runtime::{CoreContext, DomainSet};
    let slot: HookSlot<u32> = HookSlot::new("slots_tests::shadow_same_id");
    slot.register("same", 1);
    let ctx = CoreContext::for_test(DomainSet::full(), None);
    slot.register_in(ctx.extensions(), "same", 2);
    let entries = CoreContext::sync_scope(ctx, || slot.entries_context_first());
    assert_eq!(entries, vec![("same".to_string(), 2)]);
    slot.clear();
}
