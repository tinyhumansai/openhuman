use serde_json::Map;

use super::*;

// ── Controller extensions (crates above the core) ──────────────────────────

fn ext_controller(namespace: &'static str, function: &'static str) -> RegisteredController {
    fn handler(_params: Map<String, serde_json::Value>) -> ControllerFuture {
        Box::pin(async { Ok(serde_json::json!({"ext": true})) })
    }
    RegisteredController {
        schema: schema(namespace, function, vec![]),
        handler,
    }
}

/// An extension's controllers are first-class for every lookup: schema,
/// dispatch, method routing, capability facts and the namespace description.
#[tokio::test]
async fn registry_extension_is_visible_to_every_lookup_and_dispatches() {
    register_controller_extension(ControllerExtension {
        group: DomainGroup::Hosted,
        controllers: vec![ext_controller("ext_probe", "ping")],
        namespaces: &[("ext_probe", "Registry extension probe.")],
    })
    .expect("register extension");

    assert_eq!(
        rpc_method_from_parts("ext_probe", "ping").as_deref(),
        Some("openhuman.ext_probe_ping")
    );
    assert!(schema_for_rpc_method("openhuman.ext_probe_ping").is_some());
    assert!(all_controller_schemas()
        .iter()
        .any(|s| s.namespace == "ext_probe" && s.function == "ping"));
    assert_eq!(
        namespace_description("ext_probe"),
        Some("Registry extension probe.")
    );

    let result = try_invoke_registered_rpc("openhuman.ext_probe_ping", Map::new())
        .await
        .expect("extension method is dispatchable")
        .expect("handler succeeds");
    assert_eq!(result, serde_json::json!({"ext": true}));
}

/// Re-registering the identical set is a no-op; a *colliding* set (a built-in
/// method) is refused by the same drift guard the boot registry passes.
#[test]
fn registry_extension_is_idempotent_and_refuses_collisions() {
    let ext = || ControllerExtension {
        group: DomainGroup::Hosted,
        controllers: vec![ext_controller("ext_idem", "once")],
        namespaces: &[("ext_idem", "Idempotency probe.")],
    };
    register_controller_extension(ext()).expect("first registration");
    register_controller_extension(ext()).expect("identical re-registration is a no-op");
    let count = {
        let view = registry_view();
        view.iter()
            .filter(|g| g.controller.schema.namespace == "ext_idem")
            .count()
    };
    assert_eq!(count, 1, "no duplicate rows after re-registration");

    // `memory.list_files`-style collision with a built-in: pick any built-in.
    let builtin = registry()
        .first()
        .expect("built-in registry is non-empty")
        .controller
        .schema
        .clone();
    let err = register_controller_extension(ControllerExtension {
        group: DomainGroup::Hosted,
        controllers: vec![ext_controller(builtin.namespace, builtin.function)],
        namespaces: &[],
    })
    .expect_err("shadowing a built-in method must be refused");
    assert!(err.contains("duplicate"), "{err}");
}

/// The ambient `DomainSet` gates extension controllers through their group,
/// exactly like built-ins: with `hosted: false` the method is unknown.
#[tokio::test]
async fn registry_extension_is_gated_by_its_domain_group() {
    register_controller_extension(ControllerExtension {
        group: DomainGroup::Hosted,
        controllers: vec![ext_controller("ext_gate", "ping")],
        namespaces: &[],
    })
    .expect("register extension");

    let mut domains = DomainSet::full();
    domains.hosted = false;
    let ctx = CoreContext::for_test(domains, None);
    let hidden = CoreContext::scope(ctx, async {
        (
            try_invoke_registered_rpc("openhuman.ext_gate_ping", Map::new())
                .await
                .is_none(),
            schema_for_rpc_method("openhuman.ext_gate_ping").is_none(),
        )
    })
    .await;
    assert_eq!(
        hidden,
        (true, true),
        "hosted: false must hide the extension"
    );
}
