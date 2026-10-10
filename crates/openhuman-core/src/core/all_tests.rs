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
    let declared = [schema("dup", "fn", vec![]), schema("dup", "fn", vec![])];
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
    let declared = [schema(
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
    let declared = [
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
    let view = registry_view();
    let schemas = controller_schemas(&view);
    let controllers = registered_controllers(&view);
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
/// stub-facade correctness gate (see `crate::voice::stub`).
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
    const { assert!(crate::inference::INFERENCE_COMPILED_IN) };
}

/// With the `inference` feature off, the marker flips and `cpal` leaves the
/// dependency graph. The observable effect is the microphone-permission probe:
/// it reports `Unknown` rather than a real verdict, because there is no
/// audio-device API compiled in to ask. Speech-to-text is unaffected in either
/// direction — it is a hosted HTTP call now, not an in-process engine.
#[test]
#[cfg(not(feature = "inference"))]
fn inference_engine_compiled_out_when_feature_off() {
    use tinycomputer_accessibility::{detect_microphone_permission, PermissionState};
    const { assert!(!crate::inference::INFERENCE_COMPILED_IN) };
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
/// stub-facade correctness gate (see `crate::skills::stub`).
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
        !crate::web3::all_web3_agent_tools().is_empty(),
        "web3 agent tools must be present when the `web3` feature is on"
    );
}

/// With the `web3` feature off, all three domains are compiled out: their
/// controllers never enter the registry (wallet/web3/x402 RPC methods are
/// unknown-method and absent from `/schema`) and the web3 agent tools are
/// gone. This is the compile-time stub-facade correctness gate (see
/// `crate::web3::{self,wallet,x402}::stub`).
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
        crate::web3::all_web3_agent_tools().is_empty(),
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

// --- validate_params bounded integers (#6137) ----------------------------

fn bounded_schema(ty: TypeSchema) -> ControllerSchema {
    schema(
        "test",
        "fn",
        vec![FieldSchema {
            name: "order",
            ty,
            comment: "",
            required: false,
        }],
    )
}

fn order_params(value: Value) -> Map<String, Value> {
    let mut p = Map::new();
    p.insert("order".into(), value);
    p
}

const U32_RANGE: TypeSchema = TypeSchema::BoundedU64 {
    min: 0,
    max: u32::MAX as u64,
};

#[test]
fn validate_params_bounded_accepts_both_inclusive_ends() {
    let s = bounded_schema(TypeSchema::BoundedU64 { min: 1, max: 10 });
    assert!(validate_params(&s, &order_params(Value::from(1u64))).is_ok());
    assert!(validate_params(&s, &order_params(Value::from(10u64))).is_ok());
    assert!(validate_params(&s, &order_params(Value::Null)).is_ok());
}

#[test]
fn validate_params_bounded_rejects_above_max_naming_the_bound() {
    // The value `is_u64()`, so a plain `U64` declaration would have let it
    // through to the handler's `u32` deserialization.
    let s = bounded_schema(U32_RANGE);
    let err = validate_params(&s, &order_params(Value::from(4_294_967_296u64))).unwrap_err();
    assert_eq!(
        err,
        "invalid type for param 'order' in test.fn: expected unsigned integer <= 4294967295, got 4294967296"
    );
}

#[test]
fn validate_params_bounded_rejects_below_min_naming_the_bound() {
    let s = bounded_schema(TypeSchema::BoundedU64 {
        min: 1,
        max: u32::MAX as u64,
    });
    let err = validate_params(&s, &order_params(Value::from(0u64))).unwrap_err();
    assert_eq!(
        err,
        "invalid type for param 'order' in test.fn: expected unsigned integer >= 1, got 0"
    );
}

#[test]
fn validate_params_bounded_rejects_non_unsigned_values_by_kind() {
    let s = bounded_schema(U32_RANGE);
    for (value, got) in [
        (Value::from(-1i64), "number"),
        (Value::from(1.5f64), "number"),
        (Value::from("7"), "string"),
    ] {
        let err = validate_params(&s, &order_params(value)).unwrap_err();
        assert_eq!(
            err,
            format!(
                "invalid type for param 'order' in test.fn: expected unsigned integer, got {got}"
            )
        );
    }
}

#[test]
fn validate_params_bounded_applies_inside_option_and_array() {
    let opt = bounded_schema(TypeSchema::Option(Box::new(U32_RANGE)));
    assert!(validate_params(&opt, &order_params(Value::from(5u64))).is_ok());
    let err = validate_params(&opt, &order_params(Value::from(u64::MAX))).unwrap_err();
    assert!(err.ends_with(&format!("got {}", u64::MAX)), "got: {err}");

    let arr = bounded_schema(TypeSchema::Array(Box::new(TypeSchema::BoundedU64 {
        min: 0,
        max: u8::MAX as u64,
    })));
    assert!(validate_params(&arr, &order_params(serde_json::json!([0, 255]))).is_ok());
    // The offending element is named, not the whole array.
    let err = validate_params(&arr, &order_params(serde_json::json!([1, 256]))).unwrap_err();
    assert!(
        err.ends_with("expected unsigned integer <= 255, got 256"),
        "got: {err}"
    );
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
    let declared = [schema("", "fn", vec![])];
    let registered = vec![RegisteredController {
        schema: declared[0].clone(),
        handler: noop_handler,
    }];
    let err = validate_registry(&grouped(registered)).unwrap_err();
    assert!(err.contains("namespace must not be empty"));
}

#[test]
fn validate_registry_rejects_empty_function() {
    let declared = [schema("ns", "", vec![])];
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
    let declared = [schema("   ", "fn", vec![])];
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
    let workspace = tempfile::TempDir::new().expect("security policy workspace");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.action_dir = workspace.path().to_path_buf();
    config.config_path = workspace.path().join("config.toml");
    let ctx = CoreContext::for_test_with_config(DomainSet::full(), config);
    let out = CoreContext::scope(
        ctx,
        try_invoke_registered_rpc("openhuman.security_policy_info", Map::new()),
    )
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
    // The `ext_*` namespaces are registered by the extension tests running
    // concurrently in this process; ignore them so the two snapshots cannot
    // straddle a registration.
    let registered: BTreeSet<String> = all_registered_controllers()
        .into_iter()
        .filter(|c| !c.schema.namespace.starts_with("ext_"))
        .map(|c| format!("{}.{}", c.schema.namespace, c.schema.function))
        .collect();
    let declared: BTreeSet<String> = all_controller_schemas()
        .into_iter()
        .filter(|s| !s.namespace.starts_with("ext_"))
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
/// `#[cfg]` in `crates/openhuman-core/src/core/all.rs` — the aggregators simply return empty vecs.
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
// One namespace, not two: `tinyflows` registers no controllers, so `flows` is
// the gate's entire controller surface.

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
/// This is the half that proves the gate does something: here we assert the
/// controller surface.
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
    const { assert!(crate::core::http_server_status::HTTP_SERVER_COMPILED_IN) };
}

/// With the `http-server` feature off, the transport is compiled out: the
/// marker flips, `serve()` returns without binding a listener, and the
/// exclusive `socketioxide` dependency leaves the graph (`axum` remains, pulled
/// transitively by `tinychannels`). The desktop shell's compile-time assert on
/// this marker (`crates/openhuman-app/src/lib.rs`) turns a silent listener-less core
/// into a build failure (cf. voice #4901).
#[test]
#[cfg(not(feature = "http-server"))]
fn http_server_compiled_out_when_feature_off() {
    assert!(!crate::core::http_server_status::HTTP_SERVER_COMPILED_IN);
}

// ---- DomainGroup ↔ family-directory realignment ----------------------------
// The reorg (#5328) made `crates/openhuman-core/src/` one directory per family, so the
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
        ("composio", DomainGroup::Integrations),
        ("task_sources", DomainGroup::Integrations),
        // `billing` / `team` / `referral` / `announcements` (`Hosted`) are no
        // longer built in: `openhuman-tinyhumans::hosted` registers them as an
        // extension — see `registry_extension_*` below.
        ("dashboard", DomainGroup::Desktop),
        ("notification", DomainGroup::Desktop),
        ("sandbox", DomainGroup::Runtimes),
        // Mis-tagged before the realignment: these live inside a named family
        // directory but answered `Platform`, so `harness()` registered nothing
        // for them despite claiming to enable their family.
        ("ai", DomainGroup::Agent),
        ("auth", DomainGroup::Security),
        ("devices", DomainGroup::Security),
        ("workspace", DomainGroup::Config),
        ("memory", DomainGroup::Memory),
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
    // Namespaces legitimately without a family: platform/, tools/,
    // test_support/, and the `http_host` extension `openhuman-rpc` registers.
    // Anything else here is a missed tag.
    for ns in &platform {
        assert!(
            !matches!(
                *ns,
                "cron"
                    | "composio"
                    | "task_sources"
                    | "billing"
                    | "team"
                    | "referral"
                    | "announcements"
                    | "dashboard"
                    | "notification"
                    | "sandbox"
                    | "ai"
                    | "auth"
                    | "devices"
                    | "workspace"
                    | "memory"
            ),
            "namespace `{ns}` belongs to a named family but is still tagged Platform"
        );
    }
}

/// `harness()` claims agent + memory + threads + config + security. This
/// asserts the enabled families are registered.
#[test]
fn harness_preset_registers_the_families_it_claims() {
    let harness = crate::core::runtime::DomainSet::harness();
    for ns in ["ai", "auth", "devices", "workspace", "memory"] {
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
    // Still needs these: skills run on the managed runtimes, and the session
    // loop is driven by cron.
    assert!(e.runtimes, "embedded() needs the code-execution runtimes");
    assert!(e.automation, "embedded() needs cron");
    assert!(e.inference, "embedded() needs inference");
    assert!(e.integrations, "embedded() needs external integrations");
}

// ---- DomainGroup drift guards ---------------------------------------------
// `DomainGroup` has three consumers the compiler does NOT check for coverage:
// `tool_group()` (tools/ops.rs), `StoreInitPlan` and `DomainSubscriberPlan`.
// Adding a variant compiles cleanly while leaving a tool ungated or a store
// unkeyed — both of which actually happened during the realignment (#5332):
// `people`'s store keyed on a different
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
    let mut seen = [false; DomainGroup::COUNT];
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
    let aggregator: Vec<&'static str> = crate::memory::all_memory_registered_controllers()
        .iter()
        .map(|c| c.schema.function)
        .collect();
    assert_eq!(
        registered, aggregator,
        "registry order for memory.* diverges from the memory schemas aggregator"
    );
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

#[path = "all_extensions_tests.rs"]
mod extensions_tests;

#[path = "all_removed_tests.rs"]
mod removed_tests;

#[path = "all_registry_tests.rs"]
mod registry_tests;

#[path = "all_domain_plan_tests.rs"]
mod domain_plan_tests;
