use super::*;

#[tokio::test]
async fn full_registration_is_byte_identical() {
    // With no ambient CoreContext (⇒ full, no filter), the public
    // `all_registered_controllers()` must equal the raw grouped registry — same
    // length AND same rpc-method-name sequence IN ORDER. This is the DoD (1)
    // proof that wrapping every entry in a `GroupedController` + filtering by the
    // ambient DomainSet changes neither the membership nor the ordering of the
    // full() surface.
    //
    // The baseline is the raw `registry()` view rather than a checked-in method
    // snapshot (a #4808 review suggestion): `all_registered_controllers()` and
    // `registry()` are DIFFERENT code paths — the former exercises the ambient
    // filter (`group_allowed`) and re-collects, the latter is the unfiltered
    // source — so this asserts the filter is an order-preserving identity under
    // full(). A frozen snapshot would instead ossify the controller list and
    // force churn on every legitimate new controller; git history is the
    // authoritative pre-#4796 baseline for "did the raw list itself change".
    // Tests in this binary may have initialized the process default context,
    // whose DomainSet can be narrower than full(). Scope the assertion
    // explicitly so parallel test order cannot change the registry surface.
    let ctx = CoreContext::for_test(DomainSet::full(), None);
    let (filtered_methods, raw_methods) = CoreContext::scope(ctx, async {
        let filtered = all_registered_controllers()
            .iter()
            .map(|c| c.rpc_method_name())
            .collect::<Vec<_>>();
        let raw = registry_view()
            .iter()
            .map(|g| g.controller.rpc_method_name())
            .collect::<Vec<_>>();
        (filtered, raw)
    })
    .await;

    assert_eq!(
        filtered_methods.len(),
        raw_methods.len(),
        "unfiltered all_registered_controllers() must equal raw registry length"
    );
    // Ordered comparison — NOT sorted. A reordering (or a drop/add) under full()
    // would change dispatch/schema iteration order and must fail here.
    assert_eq!(
        filtered_methods, raw_methods,
        "unfiltered rpc-method sequence must be byte-identical (order + membership) to the raw registry"
    );
}
