use serde_json::Map;

use super::*;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

fn schema(
    namespace: &'static str,
    function: &'static str,
    inputs: Vec<FieldSchema>,
) -> ControllerSchema {
    ControllerSchema {
        namespace,
        function,
        description: "test",
        inputs,
        outputs: vec![],
    }
}

fn noop_handler(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async { Ok(Value::Null) })
}

/// Wrap raw controllers as [`GroupedController`]s (all `Platform`) so the
/// `validate_registry` unit tests — which build hand-made `RegisteredController`
/// lists — can feed the grouped-registry signature (#4796). The group is
/// irrelevant to `validate_registry`, which only inspects `.controller.schema`.
fn grouped(controllers: Vec<RegisteredController>) -> Vec<GroupedController> {
    controllers
        .into_iter()
        .map(|controller| GroupedController {
            group: DomainGroup::Platform,
            controller,
        })
        .collect()
}

#[test]
fn validate_registry_rejects_duplicate_namespace_function() {
    let declared = vec![schema("dup", "fn", vec![]), schema("dup", "fn", vec![])];
    let registered = vec![
        RegisteredController {
            schema: declared[0].clone(),
            handler: noop_handler,
        },
        RegisteredController {
            schema: declared[1].clone(),
            handler: noop_handler,
        },
    ];

    let err = validate_registry(&grouped(registered)).expect_err("expected duplicate error");
    assert!(err.contains("duplicate registered controller `dup.fn`"));
}

#[test]
fn validate_registry_rejects_duplicate_required_inputs() {
    let declared = vec![schema(
        "doctor",
        "models",
        vec![
            FieldSchema {
                name: "use_cache",
                ty: TypeSchema::Bool,
                comment: "x",
                required: true,
            },
            FieldSchema {
                name: "use_cache",
                ty: TypeSchema::Bool,
                comment: "x",
                required: true,
            },
        ],
    )];
    let registered = vec![RegisteredController {
        schema: declared[0].clone(),
        handler: noop_handler,
    }];

    let err = validate_registry(&grouped(registered)).expect_err("expected duplicate input");
    assert!(err.contains("duplicate required input `use_cache` in `doctor.models`"));
}

#[test]
fn validate_registry_accepts_valid_registry() {
    let declared = vec![
        schema("ns1", "fn1", vec![]),
        schema("ns1", "fn2", vec![]),
        schema("ns2", "fn1", vec![]),
    ];
    let registered = declared
        .iter()
        .map(|s| RegisteredController {
            schema: s.clone(),
            handler: noop_handler,
        })
        .collect::<Vec<_>>();
    assert!(validate_registry(&grouped(registered)).is_ok());
}

#[test]
fn rpc_method_name_formats_correctly() {
    let s = schema("memory", "doc_put", vec![]);
    assert_eq!(rpc_method_name(&s), "openhuman.memory_doc_put");
}

#[test]
fn registered_controller_rpc_method_name() {
    let s = schema("billing", "get_balance", vec![]);
    let rc = RegisteredController {
        schema: s,
        handler: noop_handler,
    };
    assert_eq!(rc.rpc_method_name(), "openhuman.billing_get_balance");
}

#[test]
fn namespace_description_known_namespaces() {
    assert!(namespace_description("memory").is_some());
    assert!(namespace_description("memory_tree").is_some());
    assert!(namespace_description("redirect_links").is_some());
    assert!(namespace_description("billing").is_some());
    assert!(namespace_description("config").is_some());
    assert!(namespace_description("health").is_some());
    assert!(namespace_description("security").is_some());
    assert!(namespace_description("tool_registry").is_some());
    assert!(namespace_description("voice").is_some());
    assert!(namespace_description("webhooks").is_some());
    assert!(namespace_description("notification").is_some());
}

#[test]
fn namespace_description_unknown_returns_none() {
    assert!(namespace_description("nonexistent_xyz").is_none());
}

#[test]
fn validate_params_accepts_valid_params() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "key",
            ty: TypeSchema::String,
            comment: "a key",
            required: true,
        }],
    );
    let mut params = Map::new();
    params.insert("key".into(), Value::String("value".into()));
    assert!(validate_params(&s, &params).is_ok());
}

#[test]
fn validate_params_rejects_missing_required() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "key",
            ty: TypeSchema::String,
            comment: "a key",
            required: true,
        }],
    );
    let params = Map::new();
    let err = validate_params(&s, &params).unwrap_err();
    assert!(err.contains("missing required param 'key'"));
}

#[test]
fn validate_params_rejects_unknown_param() {
    let s = schema("test", "fn", vec![]);
    let mut params = Map::new();
    params.insert("unknown".into(), Value::Null);
    let err = validate_params(&s, &params).unwrap_err();
    assert!(err.contains("unknown param 'unknown'"));
}

#[test]
fn validate_params_accepts_empty_for_no_required() {
    let s = schema("test", "fn", vec![]);
    assert!(validate_params(&s, &Map::new()).is_ok());
}

#[test]
fn all_registered_controllers_is_nonempty() {
    let controllers = all_registered_controllers();
    assert!(
        controllers.len() > 50,
        "expected many controllers, got {}",
        controllers.len()
    );
}

#[test]
fn all_controller_schemas_matches_registered_count() {
    let schemas = all_controller_schemas();
    let controllers = all_registered_controllers();
    assert_eq!(schemas.len(), controllers.len());
}

/// With the `voice` feature on (the default), the voice + audio_toolkit
/// controllers are compiled in and registered — the desktop build is
/// byte-identical.
#[test]
#[cfg(feature = "voice")]
fn voice_and_audio_controllers_registered_when_feature_on() {
    let schemas = all_controller_schemas();
    assert!(
        schemas.iter().any(|s| s.namespace == "voice"),
        "voice controllers must be registered when the `voice` feature is on"
    );
    assert!(
        schemas.iter().any(|s| s.namespace == "audio_toolkit"),
        "audio_toolkit controllers must be registered when the `voice` feature is on"
    );
}

/// With the `voice` feature off, both domains are compiled out: their
/// controllers never enter the registry, so voice/audio RPC methods are
/// unknown-method and absent from `/schema`. This is the compile-time
/// stub-facade correctness gate (see `openhuman::voice::stub`).
#[test]
#[cfg(not(feature = "voice"))]
fn voice_and_audio_controllers_absent_when_feature_off() {
    let schemas = all_controller_schemas();
    assert!(
        !schemas
            .iter()
            .any(|s| s.namespace == "voice" || s.namespace == "audio_toolkit"),
        "voice/audio_toolkit controllers must be compiled out when the `voice` feature is off"
    );
}

/// With the `web3` feature on (the default), the wallet + web3 + x402
/// controllers are compiled in and registered, and the high-level web3 agent
/// tools (swap/bridge/dapp) are present — the desktop build is byte-identical.
#[test]
#[cfg(feature = "web3")]
fn wallet_web3_x402_controllers_registered_when_feature_on() {
    let schemas = all_controller_schemas();
    assert!(
        schemas.iter().any(|s| s.namespace == "wallet"),
        "wallet controllers must be registered when the `web3` feature is on"
    );
    assert!(
        schemas.iter().any(|s| s.namespace.starts_with("web3_")),
        "web3 (swap/bridge/dapp) controllers must be registered when the `web3` feature is on"
    );
    assert!(
        schemas.iter().any(|s| s.namespace == "x402"),
        "x402 controllers must be registered when the `web3` feature is on"
    );
    assert!(
        !crate::openhuman::web3::all_web3_agent_tools().is_empty(),
        "web3 agent tools must be present when the `web3` feature is on"
    );
}

/// With the `web3` feature off, all three domains are compiled out: their
/// controllers never enter the registry (wallet/web3/x402 RPC methods are
/// unknown-method and absent from `/schema`) and the web3 agent tools are
/// gone. This is the compile-time stub-facade correctness gate (see
/// `openhuman::{wallet,web3,x402}::stub`).
#[test]
#[cfg(not(feature = "web3"))]
fn wallet_web3_x402_controllers_absent_when_feature_off() {
    let schemas = all_controller_schemas();
    assert!(
        !schemas.iter().any(|s| s.namespace == "wallet"
            || s.namespace.starts_with("web3_")
            || s.namespace == "x402"),
        "wallet/web3/x402 controllers must be compiled out when the `web3` feature is off"
    );
    assert!(
        crate::openhuman::web3::all_web3_agent_tools().is_empty(),
        "web3 agent tools must be gone when the `web3` feature is off"
    );
}

#[test]
fn schema_for_rpc_method_finds_known_method() {
    let schema = schema_for_rpc_method("openhuman.health_snapshot");
    assert!(schema.is_some(), "health.snapshot should be findable");
    let s = schema.unwrap();
    assert_eq!(s.namespace, "health");
    assert_eq!(s.function, "snapshot");
}

#[test]
fn schema_for_rpc_method_finds_security_policy_info() {
    let schema = schema_for_rpc_method("openhuman.security_policy_info");
    assert!(schema.is_some(), "security.policy_info should be findable");
    let s = schema.unwrap();
    assert_eq!(s.namespace, "security");
    assert_eq!(s.function, "policy_info");
}

#[test]
fn schema_for_rpc_method_finds_internal_mcp_audit_list() {
    let schema = schema_for_rpc_method("openhuman.mcp_audit_list");
    assert!(
        schema.is_some(),
        "mcp_audit.list should be internally routable"
    );
    let s = schema.unwrap();
    assert_eq!(s.namespace, "mcp_audit");
    assert_eq!(s.function, "list");
}

#[test]
fn schema_for_rpc_method_finds_internal_orchestration_pairing_link_session() {
    let schema = schema_for_rpc_method("openhuman.orchestration_pairing_link_session");
    assert!(
        schema.is_some(),
        "orchestration_pairing.link_session should be internally routable"
    );
    let s = schema.unwrap();
    assert_eq!(s.namespace, "orchestration_pairing");
    assert_eq!(s.function, "link_session");
}

#[test]
fn rpc_method_from_parts_does_not_expose_internal_mcp_audit_list() {
    assert!(
        rpc_method_from_parts("mcp_audit", "list").is_none(),
        "internal MCP audit RPC must not appear in the public controller registry"
    );
}

#[test]
fn rpc_method_from_parts_does_not_expose_internal_orchestration_pairing() {
    assert!(
        rpc_method_from_parts("orchestration_pairing", "link_session").is_none(),
        "pairing write RPCs must not appear in the public controller registry"
    );
}

#[test]
fn schema_for_rpc_method_returns_none_for_unknown() {
    assert!(schema_for_rpc_method("openhuman.nonexistent_method_xyz").is_none());
}

#[test]
fn rpc_method_from_parts_finds_known() {
    let method = rpc_method_from_parts("health", "snapshot");
    assert_eq!(method.as_deref(), Some("openhuman.health_snapshot"));
}

#[test]
fn rpc_method_from_parts_returns_none_for_unknown() {
    assert!(rpc_method_from_parts("fake", "method").is_none());
}

#[test]
fn no_duplicate_rpc_methods_in_registry() {
    let controllers = all_registered_controllers();
    let mut methods: Vec<String> = controllers.iter().map(|c| c.rpc_method_name()).collect();
    let original_len = methods.len();
    methods.sort();
    methods.dedup();
    assert_eq!(
        methods.len(),
        original_len,
        "duplicate RPC methods found in registry"
    );
}

// --- validate_params edge cases -----------------------------------------

#[test]
fn validate_params_accepts_missing_optional_param() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "filter",
            ty: TypeSchema::String,
            comment: "optional filter",
            required: false,
        }],
    );
    assert!(validate_params(&s, &Map::new()).is_ok());
}

#[test]
fn validate_params_accepts_optional_param_when_present() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "filter",
            ty: TypeSchema::String,
            comment: "",
            required: false,
        }],
    );
    let mut p = Map::new();
    p.insert("filter".into(), Value::String("abc".into()));
    assert!(validate_params(&s, &p).is_ok());
}

#[test]
fn validate_params_missing_required_error_includes_comment() {
    // The comment text helps callers (esp. the CLI/UI) understand what
    // the missing field is for — lock this in so error messages don't
    // regress to bare field names.
    let s = schema(
        "memory",
        "doc_put",
        vec![FieldSchema {
            name: "namespace",
            ty: TypeSchema::String,
            comment: "namespace to write into",
            required: true,
        }],
    );
    let err = validate_params(&s, &Map::new()).unwrap_err();
    assert!(err.contains("missing required param 'namespace'"));
    assert!(err.contains("namespace to write into"));
}

#[test]
fn validate_params_unknown_error_includes_namespace_and_function() {
    let s = schema("billing", "top_up", vec![]);
    let mut p = Map::new();
    p.insert("typo".into(), Value::Null);
    let err = validate_params(&s, &p).unwrap_err();
    assert!(err.contains("unknown param 'typo'"));
    assert!(err.contains("billing.top_up"));
}

#[test]
fn validate_params_reports_missing_required_before_unknown() {
    // If a call both omits a required param AND has an unknown one,
    // the missing-required error fires first (it's strictly more
    // actionable for callers).
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "key",
            ty: TypeSchema::String,
            comment: "",
            required: true,
        }],
    );
    let mut p = Map::new();
    p.insert("unknown".into(), Value::Null);
    let err = validate_params(&s, &p).unwrap_err();
    assert!(err.contains("missing required param 'key'"), "got: {err}");
}

#[test]
fn validate_params_null_for_required_is_acceptable() {
    // JSON-RPC semantics: `null` is a valid value for an optional field
    // sent explicitly. For a required field, presence (not value) is
    // what we check — null does satisfy the "key present" check.
    // Handlers enforce stronger type contracts downstream.
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "key",
            ty: TypeSchema::String,
            comment: "",
            required: true,
        }],
    );
    let mut p = Map::new();
    p.insert("key".into(), Value::Null);
    assert!(validate_params(&s, &p).is_ok());
}

// --- validate_params type checking (C12) --------------------------------

#[test]
fn validate_params_rejects_wrong_scalar_type() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "count",
            ty: TypeSchema::U64,
            comment: "",
            required: true,
        }],
    );
    let mut p = Map::new();
    p.insert("count".into(), Value::String("nope".into()));
    let err = validate_params(&s, &p).unwrap_err();
    assert!(err.contains("invalid type for param 'count'"), "got: {err}");
    assert!(err.contains("expected unsigned integer"), "got: {err}");
}

#[test]
fn validate_params_accepts_correct_scalar_type() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "flag",
            ty: TypeSchema::Bool,
            comment: "",
            required: true,
        }],
    );
    let mut p = Map::new();
    p.insert("flag".into(), Value::Bool(true));
    assert!(validate_params(&s, &p).is_ok());
}

#[test]
fn validate_params_validates_array_element_types() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "ids",
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "",
            required: true,
        }],
    );
    let mut ok = Map::new();
    ok.insert(
        "ids".into(),
        Value::Array(vec![Value::String("a".into()), Value::String("b".into())]),
    );
    assert!(validate_params(&s, &ok).is_ok());

    let mut bad = Map::new();
    bad.insert(
        "ids".into(),
        Value::Array(vec![Value::String("a".into()), Value::Bool(true)]),
    );
    let err = validate_params(&s, &bad).unwrap_err();
    assert!(err.contains("invalid type for param 'ids'"), "got: {err}");
}

#[test]
fn validate_params_enforces_enum_variants() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "mode",
            ty: TypeSchema::Enum {
                variants: vec!["read", "write"],
            },
            comment: "",
            required: true,
        }],
    );
    let mut ok = Map::new();
    ok.insert("mode".into(), Value::String("read".into()));
    assert!(validate_params(&s, &ok).is_ok());

    let mut bad = Map::new();
    bad.insert("mode".into(), Value::String("delete".into()));
    let err = validate_params(&s, &bad).unwrap_err();
    assert!(err.contains("enum variants"), "got: {err}");
}

#[test]
fn validate_params_option_accepts_null_and_inner_type() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "limit",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "",
            required: false,
        }],
    );
    let mut null_p = Map::new();
    null_p.insert("limit".into(), Value::Null);
    assert!(validate_params(&s, &null_p).is_ok());

    let mut val_p = Map::new();
    val_p.insert("limit".into(), Value::Number(5.into()));
    assert!(validate_params(&s, &val_p).is_ok());

    let mut bad_p = Map::new();
    bad_p.insert("limit".into(), Value::String("x".into()));
    assert!(validate_params(&s, &bad_p).is_err());
}

#[test]
fn validate_params_json_type_accepts_anything() {
    let s = schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "payload",
            ty: TypeSchema::Json,
            comment: "",
            required: true,
        }],
    );
    let mut p = Map::new();
    p.insert("payload".into(), Value::Array(vec![Value::Bool(true)]));
    assert!(validate_params(&s, &p).is_ok());
}

// --- validate_registry edge cases ---------------------------------------

#[test]
fn validate_registry_rejects_empty_namespace() {
    let declared = vec![schema("", "fn", vec![])];
    let registered = vec![RegisteredController {
        schema: declared[0].clone(),
        handler: noop_handler,
    }];
    let err = validate_registry(&grouped(registered)).unwrap_err();
    assert!(err.contains("namespace must not be empty"));
}

#[test]
fn validate_registry_rejects_empty_function() {
    let declared = vec![schema("ns", "", vec![])];
    let registered = vec![RegisteredController {
        schema: declared[0].clone(),
        handler: noop_handler,
    }];
    let err = validate_registry(&grouped(registered)).unwrap_err();
    assert!(err.contains("function must not be empty"));
}

#[test]
fn validate_registry_rejects_whitespace_only_namespace() {
    // `trim().is_empty()` is the invariant — a namespace of "   " must
    // be rejected to prevent `openhuman.   _fn` nonsense RPC method names.
    let declared = vec![schema("   ", "fn", vec![])];
    let registered = vec![RegisteredController {
        schema: declared[0].clone(),
        handler: noop_handler,
    }];
    let err = validate_registry(&grouped(registered)).unwrap_err();
    assert!(err.contains("namespace must not be empty"));
}

// Note: the previous `declared_without_registered` / `registered_without_declared`
// drift tests were removed with the registry collapse (Phase 2) — schemas are now
// derived from the registered controllers, so the two lists cannot drift.

#[test]
fn validate_registry_rejects_duplicate_registered_controllers() {
    let s = schema("a", "b", vec![]);
    let registered = vec![
        RegisteredController {
            schema: s.clone(),
            handler: noop_handler,
        },
        RegisteredController {
            schema: s,
            handler: noop_handler,
        },
    ];
    let err = validate_registry(&grouped(registered)).unwrap_err();
    assert!(err.contains("duplicate registered controller `a.b`"));
}

// --- try_invoke_registered_rpc routing ---------------------------------

#[tokio::test]
async fn try_invoke_registered_rpc_returns_none_for_unknown_method() {
    let out = try_invoke_registered_rpc("openhuman.not_a_real_method_xyz_123", Map::new()).await;
    assert!(out.is_none(), "unknown methods must return None");
}

#[tokio::test]
async fn try_invoke_registered_rpc_returns_some_for_known_method() {
    // `openhuman.health_snapshot` is registered at startup and takes no
    // required params — it must route and produce Some(_).
    let out = try_invoke_registered_rpc("openhuman.health_snapshot", Map::new()).await;
    assert!(out.is_some(), "known method must route");
}

#[tokio::test]
async fn try_invoke_registered_rpc_routes_security_policy_info() {
    let out = try_invoke_registered_rpc("openhuman.security_policy_info", Map::new())
        .await
        .expect("security policy info should be registered")
        .expect("security policy info should succeed");

    assert!(
        out.get("result").is_some() || out.get("autonomy").is_some(),
        "security policy info should return policy payload: {out}"
    );
}

#[test]
fn rpc_method_name_handles_multi_underscore_function() {
    // Functions often contain underscores — the RPC method name must
    // preserve them verbatim, separated from the namespace with `_`.
    let s = schema("team", "change_member_role", vec![]);
    assert_eq!(rpc_method_name(&s), "openhuman.team_change_member_role");
}

#[test]
fn every_registered_controller_has_matching_declared_schema() {
    // Global invariant: the registry is consistent by construction.
    // This test re-asserts the contract to catch drift.
    use std::collections::BTreeSet;
    let registered: BTreeSet<String> = all_registered_controllers()
        .into_iter()
        .map(|c| format!("{}.{}", c.schema.namespace, c.schema.function))
        .collect();
    let declared: BTreeSet<String> = all_controller_schemas()
        .into_iter()
        .map(|s| format!("{}.{}", s.namespace, s.function))
        .collect();
    assert_eq!(
        registered, declared,
        "registry/schema sets must be identical"
    );
}

// --- DomainSet registration filter (#4796) ------------------------------

use crate::core::runtime::context::CoreContext;
use crate::core::runtime::DomainSet;

/// The [`DomainGroup`] a registered controller (agent-facing OR internal) is
/// tagged with, looked up by its namespace. Test-only helper over the private
/// grouped registry.
fn group_for_namespace(ns: &str) -> Option<DomainGroup> {
    registry()
        .iter()
        .chain(internal_registry().iter())
        .find(|g| g.controller.schema.namespace == ns)
        .map(|g| g.group)
}

#[test]
fn full_registration_is_byte_identical() {
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
    let filtered_methods: Vec<String> = all_registered_controllers()
        .iter()
        .map(|c| c.rpc_method_name())
        .collect();
    let raw_methods: Vec<String> = registry()
        .iter()
        .map(|g| g.controller.rpc_method_name())
        .collect();

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

#[tokio::test]
async fn harness_excludes_gated_namespaces() {
    use std::collections::BTreeSet;

    // Baseline (full, no scope) — every family present.
    let full_ns: BTreeSet<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(full_ns.contains("flows"), "full() must expose flows");
    assert!(full_ns.contains("voice"), "full() must expose voice");

    let ctx = CoreContext::for_test(DomainSet::harness(), None);
    let harness_ns: BTreeSet<&'static str> =
        CoreContext::scope(ctx, async { all_controller_schemas() })
            .await
            .iter()
            .map(|s| s.namespace)
            .collect();

    // Harness families remain.
    for present in ["memory", "threads", "config", "security", "agent"] {
        assert!(
            harness_ns.contains(present),
            "harness() must keep the `{present}` namespace"
        );
    }
    // Gate families + platform-only namespaces are gone.
    for absent in [
        "flows",
        "voice",
        "skills",
        "wallet",
        "meet",
        "mcp_clients",
        "health",
    ] {
        assert!(
            !harness_ns.contains(absent),
            "harness() must omit the gated/platform `{absent}` namespace"
        );
    }
    assert!(
        harness_ns.len() < full_ns.len(),
        "harness() must expose strictly fewer namespaces than full()"
    );
}

#[tokio::test]
async fn dispatch_returns_none_for_gated_method() {
    // A method whose group is gated OFF under the ambient DomainSet must
    // dispatch as an unknown method (None) — indistinguishable from absent.
    let gated_method = all_registered_controllers()
        .into_iter()
        .find(|c| c.schema.namespace == "flows")
        .map(|c| c.rpc_method_name())
        .expect("a flows.* method exists in the full registry");

    let ctx = CoreContext::for_test(DomainSet::harness(), None);
    let out = CoreContext::scope(ctx, try_invoke_registered_rpc(&gated_method, Map::new())).await;
    assert!(
        out.is_none(),
        "gated method `{gated_method}` must dispatch as None under harness()"
    );

    // A harness-family method still routes (Some) — security.policy_info needs
    // no workspace, so it is a clean positive control.
    let ctx = CoreContext::for_test(DomainSet::harness(), None);
    let out = CoreContext::scope(
        ctx,
        try_invoke_registered_rpc("openhuman.security_policy_info", Map::new()),
    )
    .await;
    assert!(
        out.is_some(),
        "harness-family security.policy_info must still route under harness()"
    );
}

#[tokio::test]
async fn schema_lookup_is_gated_in_lockstep_with_dispatch() {
    // #4808 review: `schema_for_rpc_method` must gate identically to
    // `try_invoke_registered_rpc`, otherwise `invoke_method_inner` validates a
    // gated method's params BEFORE the dispatch gate fires — returning the
    // controller's validation error instead of method-not-found and leaking the
    // hidden RPC surface. Prove the schema lookup returns None for a gated
    // method under harness() (so no validation runs) while a harness-family
    // method still resolves.
    let gated_method = all_registered_controllers()
        .into_iter()
        .find(|c| c.schema.namespace == "flows")
        .map(|c| c.rpc_method_name())
        .expect("a flows.* method exists in the full registry");

    // Full (no scope): the gated method's schema IS visible — proves the None
    // below is the gate, not a missing method.
    assert!(
        schema_for_rpc_method(&gated_method).is_some(),
        "under full() the schema for `{gated_method}` must resolve"
    );

    let ctx = CoreContext::for_test(DomainSet::harness(), None);
    let gated_schema =
        CoreContext::scope(ctx, async { schema_for_rpc_method(&gated_method) }).await;
    assert!(
        gated_schema.is_none(),
        "schema lookup for gated `{gated_method}` must be None under harness() (no param validation, no surface leak)"
    );

    let ctx = CoreContext::for_test(DomainSet::harness(), None);
    let kept_schema = CoreContext::scope(ctx, async {
        schema_for_rpc_method("openhuman.security_policy_info")
    })
    .await;
    assert!(
        kept_schema.is_some(),
        "harness-family security.policy_info schema must still resolve under harness()"
    );
}

#[test]
fn group_mapping_smoke() {
    // Representative controller from each harness family maps to its group…
    assert_eq!(group_for_namespace("memory"), Some(DomainGroup::Memory));
    assert_eq!(group_for_namespace("threads"), Some(DomainGroup::Threads));
    assert_eq!(group_for_namespace("config"), Some(DomainGroup::Config));
    assert_eq!(group_for_namespace("security"), Some(DomainGroup::Security));
    assert_eq!(group_for_namespace("agent"), Some(DomainGroup::Agent));
    // …and a representative gated one maps to its gate group.
    assert_eq!(group_for_namespace("flows"), Some(DomainGroup::Flows));
    assert_eq!(group_for_namespace("skills"), Some(DomainGroup::Skills));
    assert_eq!(group_for_namespace("voice"), Some(DomainGroup::Voice));
    #[cfg(feature = "web3")]
    assert_eq!(group_for_namespace("wallet"), Some(DomainGroup::Web3));
    // `meet` is compiled out under `--no-default-features`, so the registry has
    // no entry to map (#4800).
    #[cfg(feature = "meet")]
    assert_eq!(group_for_namespace("meet"), Some(DomainGroup::Meet));
    // Internal-only registry is grouped too (mcp_audit → Mcp).
    assert_eq!(group_for_namespace("mcp_audit"), Some(DomainGroup::Mcp));
}

/// All three Meet namespaces register when the `meet` feature is on (#4800).
///
/// Paired with `meet_controllers_absent_when_feature_off` below: together they
/// pin *both* directions of the compile-time gate. The negative half is the one
/// that actually proves the gate does something — a gate that never removes
/// anything would still pass this positive test.
#[cfg(feature = "meet")]
#[test]
fn meet_controllers_registered_when_feature_on() {
    for ns in ["meet", "agent_meetings", "meet_agent"] {
        assert_eq!(
            group_for_namespace(ns),
            Some(DomainGroup::Meet),
            "`{ns}` must register under DomainGroup::Meet when the `meet` feature is on"
        );
    }
}

/// No Meet namespace registers when the `meet` feature is off (#4800).
///
/// This is the half that proves the gate: with `meet` compiled out the three
/// domains must leave zero trace in either the public or the internal registry.
#[cfg(not(feature = "meet"))]
#[test]
fn meet_controllers_absent_when_feature_off() {
    for ns in ["meet", "agent_meetings", "meet_agent"] {
        assert_eq!(
            group_for_namespace(ns),
            None,
            "`{ns}` must not register when the `meet` feature is off"
        );
    }
}
