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
            capability: None,
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
    assert!(namespace_description("billing").is_some());
    assert!(namespace_description("config").is_some());
    assert!(namespace_description("health").is_some());
    assert!(namespace_description("subsystems").is_some());
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

/// With the `inference` feature on (the default), the `cpal` audio-device stack
/// is compiled in — `INFERENCE_COMPILED_IN` reflects that, and the
/// microphone-permission probe can actually inspect a device (dependency shed
/// proven separately by `cargo tree -i cpal`).
#[test]
#[cfg(feature = "inference")]
fn inference_engine_compiled_in_when_feature_on() {
    assert!(crate::openhuman::inference::INFERENCE_COMPILED_IN);
}

/// With the `inference` feature off, the marker flips and `cpal` leaves the
/// dependency graph. The observable effect is the microphone-permission probe:
/// it reports `Unknown` rather than a real verdict, because there is no
/// audio-device API compiled in to ask. Speech-to-text is unaffected in either
/// direction — it is a hosted HTTP call now, not an in-process engine.
#[test]
#[cfg(not(feature = "inference"))]
fn inference_engine_compiled_out_when_feature_off() {
    use crate::openhuman::desktop::accessibility::{detect_microphone_permission, PermissionState};
    assert!(!crate::openhuman::inference::INFERENCE_COMPILED_IN);
    assert_eq!(
        detect_microphone_permission(),
        PermissionState::Unknown,
        "without `inference` there is no audio-device API to probe"
    );
}

/// With the `skills` feature on (the default), all three skill domains are
/// compiled in and registered — the desktop build is byte-identical.
#[test]
#[cfg(feature = "skills")]
fn skill_controllers_registered_when_feature_on() {
    let schemas = all_controller_schemas();
    for ns in ["skills", "skill_runtime", "skill_registry"] {
        assert!(
            schemas.iter().any(|s| s.namespace == ns),
            "`{ns}` controllers must be registered when the `skills` feature is on"
        );
    }
}

/// With the `skills` feature off, all three domains are compiled out: their
/// controllers never enter the registry, so skills RPC methods are
/// unknown-method and absent from `/schema`. This is the compile-time
/// stub-facade correctness gate (see `openhuman::skills::stub`).
///
/// Note this does NOT cover `skills::types` / `skills::ops_types`: those stay
/// compiled in both directions (the type carve-out — `tools::traits` re-exports
/// `ToolResult`/`ToolContent` out of them), but they expose no controllers, so
/// the namespaces are absent either way.
#[test]
#[cfg(not(feature = "skills"))]
fn skill_controllers_absent_when_feature_off() {
    let schemas = all_controller_schemas();
    assert!(
        !schemas.iter().any(|s| s.namespace == "skills"
            || s.namespace == "skill_runtime"
            || s.namespace == "skill_registry"),
        "skills/skill_runtime/skill_registry controllers must be compiled out \
         when the `skills` feature is off"
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
/// `openhuman::web3::{self,wallet,x402}::stub`).
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
#[cfg(feature = "mcp")]
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
fn subsystems_namespace_is_registered_under_platform() {
    assert_eq!(
        group_for_namespace("subsystems"),
        Some(DomainGroup::Platform)
    );
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
    #[cfg(feature = "flows")]
    assert!(full_ns.contains("flows"), "full() must expose flows");
    // `voice` was the pathfinder gate (#4803) and predates this per-assert cfg
    // convention; gate it like its siblings so the disabled build passes (#5022).
    #[cfg(feature = "voice")]
    assert!(full_ns.contains("voice"), "full() must expose voice");
    #[cfg(feature = "channels")]
    assert!(full_ns.contains("channels"), "full() must expose channels");

    let ctx = CoreContext::for_test(DomainSet::harness(), None, None);
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
        "channels",
        "mcp_clients",
        "health",
        // The subsystem status surface is Platform-tagged for the same reason
        // `health` is: it is kernel operator surface with no family. An
        // embedded harness host reads driver capabilities through
        // `memory.provider_status`, which stays reachable.
        "subsystems",
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

// Uses a `flows.*` method as its gated-family vehicle, so the whole test is
// `#[cfg(feature = "flows")]`: without the feature there is no flows controller
// in the registry at all and the `.expect()` below would panic. The runtime
// gating this proves is orthogonal to the compile-time gate, and CI runs the
// test suite on default features (flows ON), so no coverage is lost there.
#[cfg(feature = "flows")]
#[tokio::test]
async fn dispatch_returns_none_for_gated_method() {
    // A method whose group is gated OFF under the ambient DomainSet must
    // dispatch as an unknown method (None) — indistinguishable from absent.
    let gated_method = all_registered_controllers()
        .into_iter()
        .find(|c| c.schema.namespace == "flows")
        .map(|c| c.rpc_method_name())
        .expect("a flows.* method exists in the full registry");

    let ctx = CoreContext::for_test(DomainSet::harness(), None, None);
    let out = CoreContext::scope(ctx, try_invoke_registered_rpc(&gated_method, Map::new())).await;
    assert!(
        out.is_none(),
        "gated method `{gated_method}` must dispatch as None under harness()"
    );

    // A harness-family method still routes (Some) — security.policy_info needs
    // no workspace, so it is a clean positive control.
    let ctx = CoreContext::for_test(DomainSet::harness(), None, None);
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

// Same flows-vehicle reasoning as `dispatch_returns_none_for_gated_method`.
#[cfg(feature = "flows")]
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

    let ctx = CoreContext::for_test(DomainSet::harness(), None, None);
    let gated_schema =
        CoreContext::scope(ctx, async { schema_for_rpc_method(&gated_method) }).await;
    assert!(
        gated_schema.is_none(),
        "schema lookup for gated `{gated_method}` must be None under harness() (no param validation, no surface leak)"
    );

    let ctx = CoreContext::for_test(DomainSet::harness(), None, None);
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
    assert_eq!(group_for_namespace("plan_review"), Some(DomainGroup::Agent));
    // …and a representative gated one maps to its gate group. `group_for_namespace`
    // reads the real controller registry, so a compile-time-gated family has no
    // entry to map when its feature is off.
    #[cfg(feature = "flows")]
    assert_eq!(group_for_namespace("flows"), Some(DomainGroup::Flows));
    // `group_for_namespace` is registry-derived, so a compile-time-gated domain
    // has no controller to map. Skip when its Cargo feature is off.
    #[cfg(feature = "skills")]
    assert_eq!(group_for_namespace("skills"), Some(DomainGroup::Skills));
    // `voice` predates the per-assert cfg convention (#4803); registry-derived, so
    // it has no entry to map when the feature is off. Gate like its siblings (#5022).
    #[cfg(feature = "voice")]
    assert_eq!(group_for_namespace("voice"), Some(DomainGroup::Voice));
    #[cfg(feature = "web3")]
    assert_eq!(group_for_namespace("wallet"), Some(DomainGroup::Web3));
    // `meet` is compiled out under `--no-default-features`, so the registry has
    // no entry to map (#4800).
    #[cfg(feature = "meet")]
    assert_eq!(group_for_namespace("meet"), Some(DomainGroup::Meet));
    // Internal-only registry is grouped too (mcp_audit → Mcp).
    // Compiled out with the `mcp` feature: `group_for_namespace` reads the LIVE
    // registry, and the gate unregisters the mcp_audit controller entirely.
    #[cfg(feature = "mcp")]
    assert_eq!(group_for_namespace("mcp_audit"), Some(DomainGroup::Mcp));
}

// --- `mcp` compile-time gate (#4799) ------------------------------------

/// With the `mcp` feature ON (the default / shipped desktop build), both MCP
/// namespaces are registered: `mcp_clients` (the dynamic Smithery registry,
/// agent-facing) and `mcp_audit` (the write-audit log, internal-only).
///
/// Paired with `mcp_namespaces_absent_when_gate_off` below so the gate is
/// pinned in BOTH directions — an assert that only ever runs in one build
/// configuration cannot prove a gate works.
#[test]
#[cfg(feature = "mcp")]
fn mcp_namespaces_registered_when_gate_on() {
    assert_eq!(
        group_for_namespace("mcp_clients"),
        Some(DomainGroup::Mcp),
        "with `mcp` compiled in, the dynamic registry's `mcp_clients` \
         namespace must be registered"
    );
    assert_eq!(
        group_for_namespace("mcp_audit"),
        Some(DomainGroup::Mcp),
        "with `mcp` compiled in, the internal `mcp_audit` namespace must be \
         registered"
    );
}

/// With the `mcp` feature OFF, both MCP namespaces are gone from the live
/// registry — every `openhuman.mcp_clients_*` / `openhuman.mcp_audit_*` method
/// is an unknown method over `/rpc` and absent from `/schema`.
///
/// This is the compile-time analogue of the runtime `DomainSet::mcp` filter:
/// `DomainSet` can hide these namespaces at runtime, this feature removes the
/// code that backs them altogether. Note the stubs make this work with NO
/// `#[cfg]` in `src/core/all.rs` — the aggregators simply return empty vecs.
#[test]
#[cfg(not(feature = "mcp"))]
fn mcp_namespaces_absent_when_gate_off() {
    assert_eq!(
        group_for_namespace("mcp_clients"),
        None,
        "with `mcp` compiled out, the `mcp_clients` namespace must not be \
         registered — the stub aggregator returns an empty vec"
    );
    assert_eq!(
        group_for_namespace("mcp_audit"),
        None,
        "with `mcp` compiled out, the internal `mcp_audit` namespace must not \
         be registered — the stub aggregator returns an empty vec"
    );
}

// --- #4797: `flows` compile-time gate (directional proof) -------------------
//
// One namespace, not three: `tinyflows` registers no controllers, and
// `rhai_workflows` is `scope() = AgentOnly` (no controller schemas in v1), so
// `flows` is the gate's entire controller surface.

#[cfg(feature = "flows")]
#[test]
fn flows_controllers_registered_when_feature_on() {
    let namespaces: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        namespaces.contains(&"flows"),
        "with the `flows` feature ON the flows controllers must be registered"
    );
}

#[cfg(not(feature = "flows"))]
#[test]
fn flows_controllers_absent_when_feature_off() {
    let namespaces: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        !namespaces.contains(&"flows"),
        "with the `flows` feature OFF the flows controllers must be absent \
         (unknown-method over /rpc, omitted from /schema)"
    );
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

/// The `modules` namespace registers when the `modules` feature is on.
#[cfg(feature = "modules")]
#[test]
fn modules_controllers_registered_when_feature_on() {
    assert_eq!(
        group_for_namespace("modules"),
        Some(DomainGroup::Modules),
        "`modules` must register under DomainGroup::Modules when the feature is on"
    );
}

/// The `modules` namespace is absent when the `modules` feature is off.
///
/// The half that proves the gate. It matters more than the usual both-ways pair,
/// because what this feature compiles in is a `dlopen` loader: a build that opted
/// out must have no way to reach one, not a loader that merely refuses.
#[cfg(not(feature = "modules"))]
#[test]
fn modules_controllers_absent_when_feature_off() {
    assert_eq!(
        group_for_namespace("modules"),
        None,
        "`modules` must leave no trace in the registry when the feature is off"
    );
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

/// The external-channel namespace registers when the `channels` feature is on
/// (#4801).
///
/// Paired with `channels_controllers_absent_when_feature_off` below to pin both
/// directions of the compile-time gate. The webview API/notification bridges
/// and WhatsApp store have moved to the Tauri shell and expose no core
/// controllers.
#[cfg(feature = "channels")]
#[test]
fn channels_controllers_registered_when_feature_on() {
    let namespaces: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        namespaces.contains(&"channels"),
        "with the `channels` feature ON the `channels` controllers must be registered"
    );
}

/// With `channels` compiled out the channel + webview-bridge domains leave zero
/// trace in the registry (#4801) — while the in-app web chat (`channel`
/// namespace) stays present, pinning the #5002 decoupling: turning off external
/// messaging must NOT take down core in-app chat.
///
/// This is the half that proves the gate does something. The 3 `whatsapp_data`
/// agent tools are pinned separately in `tools::ops_tests` (that module has the
/// full-tool-list machinery); here we assert the controller surface.
#[cfg(not(feature = "channels"))]
#[test]
fn channels_controllers_absent_when_feature_off() {
    let namespaces: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        !namespaces.contains(&"channels"),
        "with the `channels` feature OFF the `channels` controllers must be absent \
         (unknown-method over /rpc, omitted from /schema)"
    );
    // #5002 decoupling: the in-app web chat controllers (RPC namespace `channel`)
    // are core product surface and must survive the `channels` gate being off.
    assert!(
        namespaces.contains(&"channel"),
        "the in-app web_chat controllers (`channel` namespace) must stay registered \
         even with the `channels` feature OFF (#5002 decoupling)"
    );
}

/// With the `http-server` feature on (the default), the HTTP + Socket.IO
/// transport is compiled in — `HTTP_SERVER_COMPILED_IN` reflects that, and
/// `socketioxide` is linked. `socketioxide` is the only dependency this gate
/// actually sheds (proven by `cargo tree -i socketioxide`); `axum` stays in the
/// graph either way because `tinychannels` pulls it transitively.
#[test]
#[cfg(feature = "http-server")]
fn http_server_compiled_in_when_feature_on() {
    assert!(crate::core::http_server_status::HTTP_SERVER_COMPILED_IN);
}

/// With the `http-server` feature off, the transport is compiled out: the
/// marker flips, `serve()` returns without binding a listener, and the
/// exclusive `socketioxide` dependency leaves the graph (`axum` remains, pulled
/// transitively by `tinychannels`). The desktop shell's compile-time assert on
/// this marker (`app/src-tauri/src/lib.rs`) turns a silent listener-less core
/// into a build failure (cf. voice #4901).
#[test]
#[cfg(not(feature = "http-server"))]
fn http_server_compiled_out_when_feature_off() {
    assert!(!crate::core::http_server_status::HTTP_SERVER_COMPILED_IN);
}

/// With `http-server` on, the `http_host` static-directory server registers its
/// controllers, so the `http_host.*` RPC surface is present in `/schema`.
#[test]
#[cfg(feature = "http-server")]
fn http_host_controllers_registered_when_http_server_on() {
    let schemas = all_controller_schemas();
    assert!(
        schemas.iter().any(|s| s.namespace == "http_host"),
        "`http_host` controllers must be registered when the `http-server` feature is on"
    );
}

/// With `http-server` off, the whole `http_host` axum domain is compiled out and
/// its controller-registration push in `core::all` is gated in lockstep, so the
/// `http_host` namespace never enters the registry (unknown-method over `/rpc`,
/// absent from `/schema`). This is the negative half that proves the gate
/// removes the surface.
#[test]
#[cfg(not(feature = "http-server"))]
fn http_host_controllers_absent_when_http_server_off() {
    let schemas = all_controller_schemas();
    assert!(
        !schemas.iter().any(|s| s.namespace == "http_host"),
        "`http_host` controllers must be compiled out when the `http-server` feature is off"
    );
}

/// The `medulla` namespace registers under `DomainGroup::Medulla` when the
/// `medulla` feature is on.
///
/// Paired with the negative below. On its own this proves nothing about the
/// gate — a gate that removed nothing would still pass it.
#[cfg(feature = "medulla")]
#[test]
fn medulla_controllers_registered_when_feature_on() {
    assert_eq!(
        group_for_namespace("medulla"),
        Some(DomainGroup::Medulla),
        "`medulla` must register under DomainGroup::Medulla when the feature is on"
    );
}

/// The `medulla` namespace leaves no trace when the feature is off.
///
/// This is the half that proves the gate removes something. It also pins the
/// intended off-state: **absence**, so a host sees unknown-method and hides the
/// surface, rather than a registered controller that fails at call time.
#[cfg(not(feature = "medulla"))]
#[test]
fn medulla_controllers_absent_when_feature_off() {
    assert_eq!(
        group_for_namespace("medulla"),
        None,
        "`medulla` must not register when the feature is off"
    );
}

// ---- DomainGroup ↔ family-directory realignment ----------------------------
// The reorg (#5328) made `src/openhuman/` one directory per family, so the
// runtime axis can finally name each one instead of sweeping half the surface
// into `Platform`. These pin that alignment in both directions.

/// Every namespace whose family got carved out of `Platform` must now report its
/// own group. Before the realignment each of these answered `Platform`, so a
/// `DomainSet` that disabled the family still served its RPC surface.
#[test]
fn carved_out_families_report_their_own_group() {
    let cases: &[(&str, DomainGroup)] = &[
        #[cfg(feature = "flows")]
        ("flows", DomainGroup::Flows),
        ("cron", DomainGroup::Automation),
        ("heartbeat", DomainGroup::Automation),
        ("composio", DomainGroup::Integrations),
        ("task_sources", DomainGroup::Integrations),
        ("billing", DomainGroup::Hosted),
        ("team", DomainGroup::Hosted),
        ("tinyplace", DomainGroup::Relay),
        ("dashboard", DomainGroup::Desktop),
        ("notification", DomainGroup::Desktop),
        ("sandbox", DomainGroup::Runtimes),
        // Mis-tagged before the realignment: these live inside a named family
        // directory but answered `Platform`, so `harness()` registered nothing
        // for them despite claiming to enable their family.
        ("agentbox", DomainGroup::Agent),
        ("harness_init", DomainGroup::Agent),
        ("ai", DomainGroup::Agent),
        ("auth", DomainGroup::Security),
        ("devices", DomainGroup::Security),
        ("workspace", DomainGroup::Config),
        ("people", DomainGroup::Memory),
    ];
    for (ns, want) in cases {
        match group_for_namespace(ns) {
            Some(got) => assert_eq!(
                got, *want,
                "namespace `{ns}` must be tagged {want:?}, got {got:?} — the DomainGroup \
                 tag has drifted from the family directory it lives in"
            ),
            None => panic!("namespace `{ns}` is not registered; update this test if it moved"),
        }
    }
}

/// `Platform` is now only the kernel surfaces with no family of their own. If a
/// namespace from a named family lands here, its `push(...)` tag was missed.
#[test]
fn platform_holds_only_kernel_surfaces() {
    let platform: Vec<&str> = registry()
        .iter()
        .chain(internal_registry().iter())
        .filter(|g| g.group == DomainGroup::Platform)
        .map(|g| g.controller.schema.namespace)
        .collect();
    // Namespaces legitimately without a family: platform/, tools/, http_host/,
    // test_support/. Anything else here is a missed tag.
    for ns in &platform {
        assert!(
            !matches!(
                *ns,
                "cron"
                    | "heartbeat"
                    | "composio"
                    | "task_sources"
                    | "billing"
                    | "team"
                    | "referral"
                    | "announcements"
                    | "tinyplace"
                    | "dashboard"
                    | "notification"
                    | "sandbox"
                    | "agentbox"
                    | "harness_init"
                    | "ai"
                    | "auth"
                    | "devices"
                    | "workspace"
                    | "people"
            ),
            "namespace `{ns}` belongs to a named family but is still tagged Platform"
        );
    }
}

/// `harness()` claims agent + memory + threads + config + security. Before the
/// realignment it silently dropped several of their namespaces into `Platform`,
/// most damagingly `harness_init` — an agent harness that never runs harness
/// init. This asserts the claim is now true.
#[test]
fn harness_preset_registers_the_families_it_claims() {
    let harness = crate::core::runtime::DomainSet::harness();
    for ns in [
        "agentbox",
        "harness_init",
        "ai",
        "auth",
        "devices",
        "workspace",
        "people",
    ] {
        let group =
            group_for_namespace(ns).unwrap_or_else(|| panic!("namespace `{ns}` is not registered"));
        assert!(
            harness.allows(group),
            "harness() must allow `{ns}` ({group:?}) — it is part of a harness family"
        );
    }
}

/// `kernel()` is the floor: threads/config/security only. It must NOT pull in
/// the two big replaceable subsystems, nor any carved-out family.
#[test]
fn kernel_preset_is_the_floor() {
    let k = crate::core::runtime::DomainSet::kernel();
    assert!(
        k.threads && k.config && k.security,
        "kernel keeps the floor"
    );
    assert!(
        !k.agent && !k.memory,
        "kernel() must not enable agent/memory — a host opts those in explicitly"
    );
    for (name, on) in [
        ("inference", k.inference),
        ("integrations", k.integrations),
        ("automation", k.automation),
        ("runtimes", k.runtimes),
        ("desktop", k.desktop),
        ("hosted", k.hosted),
        ("relay", k.relay),
        ("platform", k.platform),
    ] {
        assert!(!on, "kernel() must leave `{name}` off");
    }
}

/// An embedded host supplies its own UI and never dials the hosted backend.
/// Before the realignment `embedded()` had to set `platform: true` to reach
/// credentials/config, which dragged both surfaces in.
#[test]
fn embedded_preset_excludes_desktop_and_hosted() {
    let e = crate::core::runtime::DomainSet::embedded();
    assert!(!e.desktop, "embedded() must not enable desktop surfaces");
    assert!(
        !e.hosted,
        "embedded() must not enable hosted-backend clients"
    );
    assert!(!e.relay, "embedded() must not enable the relay surface");
    // Still needs these: skills run on the managed runtimes, and the session
    // loop is driven by cron/heartbeat.
    assert!(e.runtimes, "embedded() needs the code-execution runtimes");
    assert!(e.automation, "embedded() needs cron + subconscious");
    assert!(e.inference, "embedded() needs inference");
    assert!(e.integrations, "embedded() needs external integrations");
}

// ---- DomainGroup drift guards ---------------------------------------------
// `DomainGroup` has three consumers the compiler does NOT check for coverage:
// `tool_group()` (tools/ops.rs), `StoreInitPlan` and `DomainSubscriberPlan`.
// Adding a variant compiles cleanly while leaving a tool ungated or a store
// unkeyed — both of which actually happened during the realignment (#5332):
// `harness_init` stayed in Platform, and `people`'s store keyed on a different
// group than its controllers, which would have served an RPC surface with no
// store behind it. These tests close that gap.

/// First link in the chain: `ALL` really does list every variant.
///
/// `DomainGroup::index` is an exhaustive match, so a new variant is a compile
/// error there first; this then fails until it is added to `ALL` and `COUNT` is
/// bumped. Every guard below iterates `ALL`, so they are only as trustworthy as
/// this test.
#[test]
fn domain_group_all_lists_every_variant() {
    assert_eq!(
        DomainGroup::ALL.len(),
        DomainGroup::COUNT,
        "DomainGroup::ALL and DomainGroup::COUNT disagree — a variant was added \
         to one but not the other"
    );
    let mut seen = vec![false; DomainGroup::COUNT];
    for g in DomainGroup::ALL {
        let i = g.index();
        assert!(
            i < DomainGroup::COUNT,
            "{g:?} has index {i} but COUNT is {} — bump COUNT",
            DomainGroup::COUNT
        );
        assert!(!seen[i], "two variants share index {i}");
        seen[i] = true;
    }
    let missing: Vec<usize> = seen
        .iter()
        .enumerate()
        .filter(|(_, s)| !**s)
        .map(|(i, _)| i)
        .collect();
    assert!(
        missing.is_empty(),
        "DomainGroup::ALL is missing the variant(s) at index {missing:?} — \
         `index()` knows about them but `ALL` does not"
    );
}

/// Every group must be a decision in `StoreInitPlan`: either it owns a store
/// field, or it is explicitly declared store-less here. A new family that owns
/// a store but is not keyed will fail this until it is listed.
#[test]
fn every_domain_group_is_accounted_for_in_store_init_plan() {
    use crate::core::runtime::context::StoreInitPlan;

    // Groups that own a store field in StoreInitPlan.
    const OWNS_STORE: &[DomainGroup] =
        &[DomainGroup::Memory, DomainGroup::Agent, DomainGroup::Skills];
    // Groups with no store of their own. Adding a variant forces a choice
    // between these two lists — that is the point.
    const STORELESS: &[DomainGroup] = &[
        DomainGroup::Threads,
        DomainGroup::Config,
        DomainGroup::Security,
        DomainGroup::Flows,
        DomainGroup::Mcp,
        DomainGroup::Meet,
        DomainGroup::Channels,
        DomainGroup::Web3,
        DomainGroup::Voice,
        DomainGroup::Media,
        DomainGroup::Medulla,
        DomainGroup::Inference,
        DomainGroup::Integrations,
        DomainGroup::Automation,
        DomainGroup::Runtimes,
        DomainGroup::Desktop,
        DomainGroup::Hosted,
        DomainGroup::Relay,
        // The registry is a compiled-in `const` table and the loaded-module set
        // lives in tinybus's own `ModuleHost`, so there is nothing for
        // `init_stores` to stand up.
        DomainGroup::Modules,
        DomainGroup::Platform,
    ];

    for g in DomainGroup::ALL {
        let owns = OWNS_STORE.contains(g);
        let storeless = STORELESS.contains(g);
        assert!(
            owns ^ storeless,
            "{g:?} is in neither (or both) of OWNS_STORE / STORELESS — decide \
             whether it needs a StoreInitPlan field and list it in exactly one"
        );
    }

    // And the owning groups actually gate their field: turning the group off
    // must turn the store off.
    let mut only_memory = crate::core::runtime::DomainSet::none();
    only_memory.memory = true;
    let plan = StoreInitPlan::for_domains(only_memory);
    assert!(plan.memory, "Memory on ⇒ memory store initialized");
    assert!(
        plan.people,
        "Memory on ⇒ people store initialized (people lives under memory/)"
    );
    assert!(!plan.agent_attachments, "Agent off ⇒ attachments store off");
    assert!(!plan.skills_prune, "Skills off ⇒ skills prune off");
}

/// Same contract for `DomainSubscriberPlan`: every group either registers
/// subscribers or is declared subscriber-less.
#[test]
fn every_domain_group_is_accounted_for_in_subscriber_plan() {
    use crate::core::jsonrpc::DomainSubscriberPlan;

    const REGISTERS: &[DomainGroup] = &[
        DomainGroup::Platform,
        DomainGroup::Channels,
        DomainGroup::Flows,
        DomainGroup::Memory,
        DomainGroup::Meet,
        DomainGroup::Agent,
        DomainGroup::Mcp,
        DomainGroup::Integrations,
        DomainGroup::Security,
        DomainGroup::Desktop,
        DomainGroup::Skills,
    ];
    const NO_SUBSCRIBERS: &[DomainGroup] = &[
        DomainGroup::Threads,
        DomainGroup::Config,
        DomainGroup::Web3,
        DomainGroup::Voice,
        DomainGroup::Media,
        DomainGroup::Medulla,
        DomainGroup::Inference,
        DomainGroup::Automation,
        DomainGroup::Runtimes,
        DomainGroup::Hosted,
        DomainGroup::Relay,
        // Modules run on their own in-process broker, so they cannot publish a
        // `DomainEvent` and there is nothing on the core bus to subscribe to.
        DomainGroup::Modules,
    ];

    for g in DomainGroup::ALL {
        assert!(
            REGISTERS.contains(g) ^ NO_SUBSCRIBERS.contains(g),
            "{g:?} is in neither (or both) of REGISTERS / NO_SUBSCRIBERS — decide \
             whether it registers event-bus subscribers and list it in exactly one"
        );
    }

    // full() must enable every registering group; none() must enable none.
    let full = DomainSubscriberPlan::for_domains(crate::core::runtime::DomainSet::full());
    let none = DomainSubscriberPlan::for_domains(crate::core::runtime::DomainSet::none());
    assert_ne!(full, none, "full() and none() must differ");
}

/// M5.1 split `memory::all_memory_registered_controllers()` into seven
/// per-family pairs pushed separately in `build_registered_controllers`. This
/// pins the observable result: the `memory` namespace still occupies one
/// contiguous run in the registry, in the aggregator's exact order. A stray
/// push (wrong place, wrong order, a family dropped) fails here.
#[test]
fn memory_controllers_form_one_contiguous_run_in_aggregator_order() {
    let all = all_registered_controllers();
    let positions: Vec<usize> = all
        .iter()
        .enumerate()
        .filter(|(_, c)| c.schema.namespace == "memory")
        .map(|(i, _)| i)
        .collect();

    assert!(!positions.is_empty(), "no memory controllers registered");
    let first = positions[0];
    let expected_run: Vec<usize> = (first..first + positions.len()).collect();
    assert_eq!(
        positions, expected_run,
        "memory controllers are no longer contiguous in the registry"
    );

    let registered: Vec<&'static str> = positions.iter().map(|&i| all[i].schema.function).collect();
    let aggregator: Vec<&'static str> =
        crate::openhuman::memory::all_memory_registered_controllers()
            .iter()
            .map(|c| c.schema.function)
            .collect();
    assert_eq!(
        registered, aggregator,
        "registry order for memory.* diverges from the memory schemas aggregator"
    );
}

// --- M5.2: memory-capability registration filter ---------------------------
//
// The capability axis is the same shape as the DomainSet axis above: the
// registry holds every controller, and the ambient `CoreContext` decides at
// READ time which ones exist. A family the bound driver never advertised is
// ABSENT — unknown-method over `/rpc`, omitted from `/schema` — rather than
// present and failing, because a registered-but-failing method teaches a model
// the capability exists and makes it retry.

use tinycortex_api::capabilities::Capability;

/// A workspace path unique to one test.
///
/// `memory::binding::BINDINGS` is a process-global `HashMap<PathBuf, _>` that
/// never evicts, so the FIRST test to bind a path fixes that path's driver for
/// every later test in the process. Sharing a path between an ON test and an
/// OFF test would make one of them silently assert the other's driver.
fn caps_ws(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/tmp/oh-m5-caps-{name}"))
}

/// `[subsystems.memory] driver = "null"` — the only narrowed capability set a
/// test can reach without booting.
///
/// `CoreContext::for_test` takes the memory *config*, not a `Capabilities`, on
/// purpose (see its doc comment): injecting a set directly would let a test
/// assert a set no driver could have advertised and would bypass the very
/// `admit` + `capabilities()` path being proven. `admit` maps `"null"` to
/// `NullMemoryProvider`, whose advertised set is exactly
/// `Capabilities::mandatory()` = {core, recall, portability} — so every
/// optional family is OFF at once. The OFF half of each pair below therefore
/// reads "absent under a driver that advertises nothing optional", not "absent
/// with only this one family missing".
fn null_driver_cfg() -> crate::openhuman::config::schema::MemorySubsystemConfig {
    crate::openhuman::config::schema::MemorySubsystemConfig {
        driver: "null".into(),
        ..Default::default()
    }
}

/// Namespaces registered under [`DomainGroup::Memory`], each with the capability
/// its registration site tags it with. The `memory` namespace is absent here —
/// it spans four families plus host surface and is covered per-function by
/// [`MEMORY_FUNCTION_CAPABILITY`].
const MEMORY_NAMESPACE_CAPABILITY: &[(&str, Option<Capability>)] = &[
    // Host-owned address book, not a driver family.
    ("people", None),
    ("memory_goals", Some(Capability::Goals)),
    // Both the tree registry and the retrieval layer share this namespace.
    ("memory_tree", Some(Capability::Tree)),
    ("tree_summarizer", Some(Capability::Tree)),
    ("slack_memory", Some(Capability::Sources)),
    ("memory_sync", Some(Capability::Sources)),
    ("memory_sources", Some(Capability::Sources)),
    #[cfg(feature = "memory-git")]
    ("memory_diff", Some(Capability::Diff)),
];

/// The `memory` namespace, function by function. Core and recall share the
/// `Core` gate, so `driver = "null"` can deliberately remove the entire
/// driver-backed memory surface. Host-only file I/O remains ungated.
const MEMORY_FUNCTION_CAPABILITY: &[(&str, Option<Capability>)] = &[
    // core + recall (both represented by the Core gate at registration)
    ("init", Some(Capability::Core)),
    ("list_documents", Some(Capability::Core)),
    ("list_namespaces", Some(Capability::Core)),
    ("delete_document", Some(Capability::Core)),
    ("query_namespace", Some(Capability::Core)),
    ("recall_context", Some(Capability::Core)),
    ("recall_memories", Some(Capability::Core)),
    ("namespace_list", Some(Capability::Core)),
    ("context_query", Some(Capability::Core)),
    ("context_recall", Some(Capability::Core)),
    ("clear_namespace", Some(Capability::Core)),
    // namespace-document tier
    ("doc_put", Some(Capability::Documents)),
    ("doc_list", Some(Capability::Documents)),
    ("doc_delete", Some(Capability::Documents)),
    // driver-owned ingestion
    ("doc_ingest", Some(Capability::Ingest)),
    // plain workspace file I/O, host-side
    ("list_files", None),
    ("read_file", None),
    ("write_file", None),
    // key/value + knowledge graph
    ("kv_set", Some(Capability::Graph)),
    ("kv_get", Some(Capability::Graph)),
    ("kv_delete", Some(Capability::Graph)),
    ("kv_list_namespace", Some(Capability::Graph)),
    ("graph_upsert", Some(Capability::Graph)),
    ("graph_query", Some(Capability::Graph)),
    // source sync
    ("sync_channel", Some(Capability::Sources)),
    ("sync_all", Some(Capability::Sources)),
    ("ingestion_status", Some(Capability::Sources)),
    // the tree summarizer, NOT ingestion
    ("learn_all", Some(Capability::Tree)),
    // never gated: this is the RPC that reports the capability set
    ("provider_status", None),
    // per-tool learned memory
    ("tool_rule_put", Some(Capability::ToolMemory)),
    ("tool_rule_get", Some(Capability::ToolMemory)),
    ("tool_rule_list", Some(Capability::ToolMemory)),
    ("tool_rule_delete", Some(Capability::ToolMemory)),
    ("tool_rules_for_prompt", Some(Capability::ToolMemory)),
    ("tool_rules_json", Some(Capability::ToolMemory)),
];

fn expected_capability(ns: &str, function: &str) -> Option<Option<Capability>> {
    if ns == "memory" {
        return MEMORY_FUNCTION_CAPABILITY
            .iter()
            .find(|(f, _)| *f == function)
            .map(|(_, c)| *c);
    }
    MEMORY_NAMESPACE_CAPABILITY
        .iter()
        .find(|(n, _)| *n == ns)
        .map(|(_, c)| *c)
}

/// Drift guard: every `DomainGroup::Memory` controller carries a
/// checked-in capability decision, and the live tag matches it.
///
/// This is what makes an untagged Memory push a test failure rather than a
/// silent `None`. `push` delegates to `push_cap(.., None, ..)`, so a new Memory
/// site added with the wrong helper compiles fine and gates nothing — only this
/// table catches it.
#[test]
fn memory_capability_map_is_exhaustive() {
    for g in registry().iter().chain(internal_registry().iter()) {
        if g.group != DomainGroup::Memory {
            continue;
        }
        let ns = g.controller.schema.namespace;
        let function = g.controller.schema.function;
        let expected = expected_capability(ns, function).unwrap_or_else(|| {
            panic!(
                "`{ns}.{function}` is registered under DomainGroup::Memory but carries no \
                 checked-in capability decision — add it to MEMORY_NAMESPACE_CAPABILITY or \
                 MEMORY_FUNCTION_CAPABILITY and tag its push site with push_cap(..)"
            )
        });
        assert_eq!(
            g.capability, expected,
            "`{ns}.{function}` is tagged {:?} at its registration site but the map says {expected:?}",
            g.capability
        );
    }
}

/// The other direction: no table entry may name a namespace/function that is no
/// longer registered, so a deleted controller cannot leave a stale decision
/// behind that looks like coverage.
#[test]
fn memory_capability_map_has_no_stale_entries() {
    let live: Vec<(&str, &str)> = registry()
        .iter()
        .chain(internal_registry().iter())
        .filter(|g| g.group == DomainGroup::Memory)
        .map(|g| (g.controller.schema.namespace, g.controller.schema.function))
        .collect();

    for (ns, _) in MEMORY_NAMESPACE_CAPABILITY {
        // `memory_diff` only registers when `memory-git` is compiled in; no CI
        // lane enables it, so it would otherwise read as a stale table entry.
        if *ns == "memory_diff" && !cfg!(feature = "memory-git") {
            continue;
        }
        assert!(
            live.iter().any(|(n, _)| n == ns),
            "MEMORY_NAMESPACE_CAPABILITY names `{ns}`, which registers no Memory controller"
        );
    }
    for (function, _) in MEMORY_FUNCTION_CAPABILITY {
        assert!(
            live.iter().any(|(n, f)| *n == "memory" && f == function),
            "MEMORY_FUNCTION_CAPABILITY names `memory.{function}`, which is not registered"
        );
    }
}

/// Every capability family is accounted for in the RPC surface — either it
/// gates at least one controller, or it is listed as deliberately RPC-less.
///
/// `Capability` is deliberately NOT `#[non_exhaustive]` (see that module's
/// docs), so a fourteenth family is a **compile error** in the `match` below
/// before it is a test failure. That compile error is the mechanism which
/// guarantees a new family gets wired somewhere rather than silently defaulting
/// to ungated.
#[test]
fn every_capability_family_is_accounted_for_in_the_rpc_surface() {
    let gated: std::collections::BTreeSet<Capability> = registry()
        .iter()
        .chain(internal_registry().iter())
        .filter_map(|g| g.capability)
        .collect();

    for cap in Capability::ALL {
        let has_rpc_surface = match cap {
            // Gate at least one controller today.
            Capability::Ingest
            | Capability::Documents
            | Capability::Tree
            | Capability::Graph
            | Capability::Goals
            | Capability::ToolMemory
            | Capability::Sources => true,
            #[cfg(feature = "memory-git")]
            Capability::Diff => true,
            #[cfg(not(feature = "memory-git"))]
            Capability::Diff => false,
            // `Core` gates the combined core + recall controller partition so
            // a null driver removes the entire driver-backed surface. Recall
            // is represented by that same partition; Portability is RPC-less.
            Capability::Core => true,
            Capability::Recall | Capability::Portability => false,
            // Folded into `Tree`: the tree registry's ~25 methods span tree,
            // entities, graph and maintenance and are tagged as ONE family.
            // See the push site in `all.rs` for why that trade was chosen.
            Capability::Entities => false,
            // No controller exposes re-embed / compact / dream / doctor yet.
            Capability::Maintenance => false,
        };
        assert_eq!(
            gated.contains(&cap),
            has_rpc_surface,
            "capability `{cap}` is {} in the live registry but the table says {}",
            if gated.contains(&cap) {
                "gating controllers"
            } else {
                "gating nothing"
            },
            if has_rpc_surface {
                "it should gate something"
            } else {
                "it should gate nothing"
            },
        );
    }
}

// --- default-open: the 4000-pre-boot-test tripwire -------------------------

#[test]
fn capability_allowed_defaults_open_with_no_context() {
    // No ambient CoreContext at all. `None` is trivially allowed, and every
    // real family must be allowed too — `current_memory_capabilities()` falls
    // back to the full set. A deny-by-default here would fail every memory
    // unit test in the crate at once.
    assert!(capability_allowed(None));
    for cap in Capability::ALL {
        assert!(
            capability_allowed(Some(cap)),
            "capability `{cap}` must default OPEN with no ambient context"
        );
    }
}

#[test]
fn unbound_registration_is_byte_identical() {
    // Companion to `full_registration_is_byte_identical`: with no ambient
    // context the capability filter must be an order-preserving identity, so
    // adding the axis changed neither membership nor ordering of the unbound
    // surface.
    let filtered: Vec<String> = all_registered_controllers()
        .iter()
        .map(|c| c.rpc_method_name())
        .collect();
    let raw: Vec<String> = registry()
        .iter()
        .map(|g| g.controller.rpc_method_name())
        .collect();
    assert_eq!(filtered, raw);
}

#[tokio::test]
async fn narrowed_capabilities_do_not_narrow_the_domain_set() {
    // The two axes are independent: a null driver hides memory families, but
    // every non-Memory namespace stays exactly as `full()` had it.
    use std::collections::BTreeSet;

    let full_ns: BTreeSet<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();

    let ctx = CoreContext::for_test(
        DomainSet::full(),
        Some(caps_ws("axes")),
        Some(null_driver_cfg()),
    );
    let null_ns: BTreeSet<&'static str> =
        CoreContext::scope(ctx, async { all_controller_schemas() })
            .await
            .iter()
            .map(|s| s.namespace)
            .collect();

    for ns in ["threads", "config", "security", "agent", "tools"] {
        assert!(
            null_ns.contains(ns),
            "a narrowed memory capability set must not remove the `{ns}` namespace"
        );
    }
    assert!(null_ns.len() < full_ns.len());
}

// --- both-ways pairs, one per gated family ---------------------------------
//
// The ABSENT half of each pair is the one that proves the gate removes
// anything; a gate that never removes anything would still pass the present
// half.

/// Namespaces + `memory.*` functions visible under the given memory config.
async fn visible_under(
    ws: &str,
    cfg: Option<crate::openhuman::config::schema::MemorySubsystemConfig>,
) -> (
    std::collections::BTreeSet<&'static str>,
    std::collections::BTreeSet<&'static str>,
) {
    let ctx = CoreContext::for_test(DomainSet::full(), Some(caps_ws(ws)), cfg);
    let schemas = CoreContext::scope(ctx, async { all_controller_schemas() }).await;
    let namespaces = schemas.iter().map(|s| s.namespace).collect();
    let memory_fns = schemas
        .iter()
        .filter(|s| s.namespace == "memory")
        .map(|s| s.function)
        .collect();
    (namespaces, memory_fns)
}

#[tokio::test]
async fn memory_families_registered_when_capabilities_advertised() {
    // The embedded `tinycortex` driver (the default config) advertises
    // `Capabilities::all()`, so every gated family is present. Scoped rather
    // than unscoped so this proves a BOUND driver's set, not the unbound
    // default-open fallback.
    let (ns, fns) = visible_under("on", None).await;

    for present in [
        "memory",
        "memory_goals",
        "memory_tree",
        "tree_summarizer",
        "memory_sync",
        "memory_sources",
        "slack_memory",
        "people",
    ] {
        assert!(
            ns.contains(present),
            "`{present}` must be present under a full-capability driver"
        );
    }
    // `memory_diff` only registers when `memory-git` is compiled in.
    if cfg!(feature = "memory-git") {
        assert!(
            ns.contains("memory_diff"),
            "`memory_diff` must be present under a full-capability driver when `memory-git` is on"
        );
    }
    for present in [
        "doc_put",
        "doc_ingest",
        "kv_set",
        "graph_query",
        "sync_all",
        "learn_all",
        "tool_rule_put",
        "provider_status",
        "recall_memories",
        "list_files",
    ] {
        assert!(
            fns.contains(present),
            "`memory.{present}` must be present under a full-capability driver"
        );
    }
}

#[tokio::test]
async fn memory_families_absent_when_capabilities_not_advertised() {
    // A null driver deliberately exposes no driver-backed memory capability,
    // so the full driver-owned surface is absent at once.
    let (ns, fns) = visible_under("off", Some(null_driver_cfg())).await;

    // Whole namespaces vanish.
    for absent in [
        "memory_goals",
        "memory_tree",
        "tree_summarizer",
        "memory_sync",
        "memory_sources",
        "memory_diff",
        "slack_memory",
    ] {
        assert!(
            !ns.contains(absent),
            "`{absent}` must be ABSENT under the null driver"
        );
    }
    // Gated `memory.*` functions vanish, including the core/recall partition…
    for absent in [
        "init",
        "list_documents",
        "list_namespaces",
        "delete_document",
        "query_namespace",
        "recall_context",
        "recall_memories",
        "namespace_list",
        "context_query",
        "context_recall",
        "clear_namespace",
        "doc_put",
        "doc_list",
        "doc_delete",
        "doc_ingest",
        "kv_set",
        "kv_get",
        "kv_delete",
        "kv_list_namespace",
        "graph_upsert",
        "graph_query",
        "sync_channel",
        "sync_all",
        "ingestion_status",
        "learn_all",
        "tool_rule_put",
        "tool_rule_get",
        "tool_rule_list",
        "tool_rule_delete",
        "tool_rules_for_prompt",
        "tool_rules_json",
    ] {
        assert!(
            !fns.contains(absent),
            "`memory.{absent}` must be ABSENT under the null driver"
        );
    }
    // …while the host-owned surface stays. These are the positive controls that
    // make the assertions above the GATE rather than a collapsed registry.
    assert!(
        ns.contains("memory"),
        "the `memory` namespace itself must survive"
    );
    assert!(
        ns.contains("people"),
        "`people` is host surface with no capability — it must survive any driver"
    );
    for present in [
        // Host-side workspace file I/O.
        "list_files",
        "read_file",
        "write_file",
        // The RPC that REPORTS the capability set — gating it would hide the
        // explanation for every absence above.
        "provider_status",
    ] {
        assert!(
            fns.contains(present),
            "`memory.{present}` is host-owned and must survive the null driver"
        );
    }
}

#[tokio::test]
async fn dispatch_returns_none_for_capability_gated_method() {
    let ctx = CoreContext::for_test(
        DomainSet::full(),
        Some(caps_ws("dispatch")),
        Some(null_driver_cfg()),
    );
    let out = CoreContext::scope(
        ctx,
        try_invoke_registered_rpc("openhuman.memory_tool_rules_json", Map::new()),
    )
    .await;
    assert!(
        out.is_none(),
        "a capability-gated method must dispatch as None — indistinguishable from absent"
    );

    // Positive control in the same driver configuration.
    let ctx = CoreContext::for_test(
        DomainSet::full(),
        Some(caps_ws("dispatch")),
        Some(null_driver_cfg()),
    );
    let out = CoreContext::scope(
        ctx,
        try_invoke_registered_rpc("openhuman.memory_provider_status", Map::new()),
    )
    .await;
    assert!(
        out.is_some(),
        "ungated `memory.provider_status` must still route under the null driver"
    );
}

#[tokio::test]
async fn schema_lookup_is_gated_in_lockstep_with_capability_dispatch() {
    // If `schema_for_rpc_method` did NOT gate, `invoke_method_inner` would run
    // param validation against a hidden method and return the controller's
    // validation error instead of method-not-found — leaking the surface the
    // gate exists to hide.
    let method = "openhuman.memory_tool_rules_json";
    assert!(
        schema_for_rpc_method(method).is_some(),
        "unscoped, the schema must resolve — so the None below is the gate, not a typo"
    );

    let ctx = CoreContext::for_test(
        DomainSet::full(),
        Some(caps_ws("schema")),
        Some(null_driver_cfg()),
    );
    let gated = CoreContext::scope(ctx, async { schema_for_rpc_method(method) }).await;
    assert!(
        gated.is_none(),
        "schema lookup for a capability-gated method must be None"
    );

    let ctx = CoreContext::for_test(
        DomainSet::full(),
        Some(caps_ws("schema")),
        Some(null_driver_cfg()),
    );
    let kept = CoreContext::scope(ctx, async {
        schema_for_rpc_method("openhuman.memory_provider_status")
    })
    .await;
    assert!(
        kept.is_some(),
        "ungated provider_status schema must still resolve"
    );
}

#[tokio::test]
async fn rpc_method_from_parts_stays_unfiltered_by_capability() {
    // `rpc_method_from_parts` searches the FULL registry by design (it backs
    // param validation and CLI routing). Pinning that here so a future "make
    // every lookup consistent" change has to be a deliberate decision.
    let ctx = CoreContext::for_test(
        DomainSet::full(),
        Some(caps_ws("parts")),
        Some(null_driver_cfg()),
    );
    let out = CoreContext::scope(ctx, async {
        rpc_method_from_parts("memory", "tool_rules_json")
    })
    .await;
    assert_eq!(out.as_deref(), Some("openhuman.memory_tool_rules_json"));
}

// --- M5.4: the null-driver degradation gate (milestone definition of done) --
//
// M5.1–M5.3 built the filter; these are the end-to-end assertions that the
// WIRING is right, using the tree family as the named vehicle. They target
// `try_invoke_registered_rpc`, `schema_for_rpc_method` and
// `all_controller_schemas` — the same three functions the HTTP layer calls
// (`core::jsonrpc::invoke_method_inner` resolves the schema then dispatches;
// `/schema` renders `all_http_method_schemas()`, which extends from
// `all_controller_schemas()`). Asserting on them IS asserting on the wire
// surface; there is no more faithful vehicle available at this level, and an
// integration test under `tests/` would be strictly WEAKER — `CoreContext::for_test`
// is `#[cfg(test)] pub(crate)` and `tests/json_rpc_e2e.rs` never calls
// `CoreContext::init`, so `current()` is `None` there and the filter would
// default OPEN, proving nothing. Do not "upgrade" these into `tests/`.
//
// The agent-tool half of the DoD is pinned next to the tool machinery that owns
// the full tool list, by `optional_family_memory_tools_absent_under_the_null_driver`
// in `src/openhuman/tools/ops_tests.rs` (`memory_tree` is in its absent list).
// Same split the channels gate uses; not duplicated here.

/// `memory_tree*` is unknown-method under a driver that never advertised
/// `Capability::Tree`.
///
/// `is_none()`, never `is_err()`: `Some(Err(_))` is the registered-but-failing
/// shape `docs/specs/kernel.md` §3.3 forbids, because a method that exists and
/// fails teaches a model the capability is real and makes it retry.
#[tokio::test]
async fn null_driver_makes_tree_methods_unknown_over_rpc() {
    let method = "openhuman.memory_tree_list_chunks";

    // Positive control FIRST: unscoped (⇒ the default-open fallback) the method
    // routes. Without this the assertion below could pass because the method
    // never existed at all.
    assert!(
        try_invoke_registered_rpc(method, Map::new())
            .await
            .is_some(),
        "`{method}` must route with no ambient context (the filter defaults OPEN)"
    );

    let ctx = CoreContext::for_test(
        DomainSet::full(), // isolates the capability gate from the DomainSet gate
        Some(caps_ws("m54-tree-dispatch")),
        Some(null_driver_cfg()),
    );
    let out = CoreContext::scope(ctx, try_invoke_registered_rpc(method, Map::new())).await;
    assert!(
        out.is_none(),
        "under the `null` driver `{method}` must dispatch as None — an unadvertised \
         family is indistinguishable from an unregistered method, never a handler \
         that returns 'not implemented'"
    );
}

/// The whole `memory_tree` namespace leaves `/schema`, and the schema lookup
/// gates in lockstep with dispatch.
///
/// Asserted as a namespace SET rather than a method list on purpose:
/// `memory_tree` is the only namespace with two registration sites — the tree
/// registry (`memory::schema::definitions`) and the retrieval layer
/// (`memory::tree::retrieval::schemas`) both use `NAMESPACE = "memory_tree"` —
/// so a method-level assertion could pass having filtered only one of them.
///
/// The lockstep half is not optional: `invoke_method_inner` resolves the schema
/// and runs `validate_params` BEFORE dispatch, so a schema lookup that is not
/// gated with dispatch leaks the hidden surface as a validation error instead
/// of method-not-found.
#[tokio::test]
async fn null_driver_removes_tree_namespace_from_schema() {
    let full_ns: std::collections::BTreeSet<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        full_ns.contains("memory_tree"),
        "unscoped ⇒ default open ⇒ memory_tree present; otherwise the assertion below is vacuous"
    );

    let ctx = CoreContext::for_test(
        DomainSet::full(),
        Some(caps_ws("m54-tree-schema")),
        Some(null_driver_cfg()),
    );
    let null_ns: std::collections::BTreeSet<&str> =
        CoreContext::scope(ctx, async { all_controller_schemas() })
            .await
            .iter()
            .map(|s| s.namespace)
            .collect();

    assert!(
        !null_ns.contains("memory_tree"),
        "both `memory_tree` registration sites must be absent from /schema under the null driver"
    );
    assert!(
        null_ns.contains("memory"),
        "the mandatory core/recall surface must survive"
    );
    assert!(
        null_ns.len() < full_ns.len(),
        "the null driver must expose strictly fewer namespaces"
    );

    // Lockstep: no schema resolves for a tree method either.
    let method = "openhuman.memory_tree_list_chunks";
    assert!(
        schema_for_rpc_method(method).is_some(),
        "unscoped the schema must resolve — so the None below is the gate, not a typo"
    );
    let ctx = CoreContext::for_test(
        DomainSet::full(),
        Some(caps_ws("m54-tree-schema")),
        Some(null_driver_cfg()),
    );
    let gated = CoreContext::scope(ctx, async { schema_for_rpc_method(method) }).await;
    assert!(
        gated.is_none(),
        "schema lookup must gate in lockstep with dispatch, or param validation leaks the surface"
    );
}

/// Degradation is not a crash: the mandatory surface still stands up.
///
/// **What this does and does not prove.** A true boot needs
/// `CoreContext::init` → `Config::load_or_init`, which is async, env-dependent
/// and writes `$HOME` — not appropriate here, and `tests/` cannot scope a
/// context at all (see the module note above). What this DOES cover is the
/// failure mode that would actually take boot down: the capability filter
/// panicking inside `registry()`'s `validate_registry` (which panics on an
/// invalid registry), or narrowing the surface to empty. Stated rather than
/// overstated — an enforcement test that oversells its guarantee is worse than
/// none, because it stops people looking.
#[tokio::test]
async fn null_driver_keeps_memory_status_routable() {
    // (1) The registry builds and self-validates under the null context.
    let schemas = CoreContext::scope(
        CoreContext::for_test(
            DomainSet::full(),
            Some(caps_ws("m54-boot")),
            Some(null_driver_cfg()),
        ),
        async { all_controller_schemas() },
    )
    .await;
    assert!(
        !schemas.is_empty(),
        "degradation must not empty the controller surface"
    );

    // (2) The driver-status surface stays reachable — it is how a host reads
    //     the capability set back, so gating it would hide the degradation.
    //
    // `is_some()`, never `is_ok()`: these handlers resolve through
    // `active_memory_client`, a process global that is uninitialised in a unit
    // test, so the inner `Result` is legitimately `Err`. ROUTABILITY is the
    // property under test.
    let out = CoreContext::scope(
        CoreContext::for_test(
            DomainSet::full(),
            Some(caps_ws("m54-boot")),
            Some(null_driver_cfg()),
        ),
        try_invoke_registered_rpc("openhuman.memory_provider_status", Map::new()),
    )
    .await;
    assert!(
        out.is_some(),
        "memory.provider_status must stay routable under any driver"
    );

    // (3) The driver-owned recall surface is intentionally removed.
    let out = CoreContext::scope(
        CoreContext::for_test(
            DomainSet::full(),
            Some(caps_ws("m54-boot")),
            Some(null_driver_cfg()),
        ),
        try_invoke_registered_rpc("openhuman.memory_recall_memories", Map::new()),
    )
    .await;
    assert!(
        out.is_none(),
        "the null driver must remove the driver-backed Recall surface"
    );
}

// --- the UNFILTERED capability lookup that backs the CLI's config-fact -------
//
// `docs/specs/kernel.md` §3.3 makes the CLI the one exception to "degradation
// is absence". The exception is only implementable if something can still tell
// "no such controller" apart from "gated" after the filtered lookups have
// collapsed both into one absence. That something is `capability_for_parts`.

#[test]
fn capability_for_parts_returns_none_for_an_unregistered_controller() {
    assert!(capability_for_parts("nope", "nope").is_none());
    assert!(capability_for_parts("memory", "not_a_function").is_none());
}

#[test]
fn capability_for_parts_reports_the_registered_family_unfiltered() {
    assert_eq!(
        capability_for_parts("memory_tree", "list_chunks"),
        Some(Some(Capability::Tree))
    );
    // Registered and deliberately ungated — distinct from "not registered".
    assert_eq!(
        capability_for_parts("memory", "provider_status"),
        Some(None)
    );
}

/// The lookup that makes the whole distinction possible: it must stay
/// unfiltered while the filtered lookup right beside it hides the method.
#[tokio::test]
async fn capability_for_parts_is_not_narrowed_by_the_ambient_context() {
    let ctx = CoreContext::for_test(
        DomainSet::full(),
        Some(caps_ws("cli-cap")),
        Some(null_driver_cfg()),
    );
    let (unfiltered, filtered) = CoreContext::scope(ctx, async {
        (
            capability_for_parts("memory_tree", "list_chunks"),
            schema_for_rpc_method("openhuman.memory_tree_list_chunks"),
        )
    })
    .await;
    assert_eq!(unfiltered, Some(Some(Capability::Tree)));
    assert!(
        filtered.is_none(),
        "the filtered lookup must still hide the gated method"
    );
}

#[test]
fn sole_capability_for_namespace_reports_a_single_family_namespace() {
    assert_eq!(
        sole_capability_for_namespace("memory_tree"),
        Some(Capability::Tree)
    );
    #[cfg(feature = "memory-git")]
    assert_eq!(
        sole_capability_for_namespace("memory_diff"),
        Some(Capability::Diff)
    );
}

#[test]
fn sole_capability_for_namespace_is_none_for_mixed_and_unknown_namespaces() {
    // `memory` spans four families plus ungated host surface.
    assert_eq!(sole_capability_for_namespace("memory"), None);
    // `people` is registered under Memory but carries no capability.
    assert_eq!(sole_capability_for_namespace("people"), None);
    assert_eq!(sole_capability_for_namespace("not_a_namespace"), None);
}

// ---- runtime-node gate -----------------------------------------------------

#[test]
#[cfg(feature = "runtime-node")]
fn javascript_controllers_registered_when_feature_on() {
    let ns: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        ns.contains(&"javascript"),
        "runtime-node ON must register the `javascript` namespace"
    );
}

/// The half that proves the gate removes anything: absent, not
/// registered-and-failing.
#[test]
#[cfg(not(feature = "runtime-node"))]
fn javascript_controllers_absent_when_feature_off() {
    let ns: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        !ns.contains(&"javascript"),
        "runtime-node OFF must not register the `javascript` namespace"
    );
}

// ---- memory-git gate -------------------------------------------------------

/// `memory-git` ON: the git-backed diff surface is registered.
#[cfg(feature = "memory-git")]
#[test]
fn memory_diff_controllers_registered_when_feature_on() {
    let namespaces: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        namespaces.contains(&"memory_diff"),
        "with the `memory-git` feature ON the `memory_diff` controllers must be registered"
    );
}

/// `memory-git` OFF: `memory_diff` leaves no trace in the registry, while the
/// rest of the memory surface stays.
///
/// This is the half that proves the gate does something. The stub's schema
/// aggregators return empty vecs rather than always-erroring handlers, so the
/// namespace must be genuinely unknown-method — not present-but-broken, which
/// would still advertise itself on `/schema`.
///
/// `memory` is asserted present in the same test on purpose: the gate is
/// supposed to remove the git ledger, not the memory domain. Splitting that
/// into a separate test would let one pass while the other silently regressed.
#[cfg(not(feature = "memory-git"))]
#[test]
fn memory_diff_controllers_absent_when_feature_off() {
    let namespaces: Vec<&str> = all_controller_schemas()
        .iter()
        .map(|s| s.namespace)
        .collect();
    assert!(
        !namespaces.contains(&"memory_diff"),
        "with `memory-git` OFF the `memory_diff` controllers must not be registered, got: {namespaces:?}"
    );
    assert!(
        namespaces.contains(&"memory"),
        "the `memory-git` gate must remove the git ledger, not the memory domain"
    );
}
