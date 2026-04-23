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

    let err = validate_registry(&registered, &declared).expect_err("expected duplicate error");
    assert!(err.contains("duplicate declared controller `dup.fn`"));
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

    let err = validate_registry(&registered, &declared).expect_err("expected duplicate input");
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
    assert!(validate_registry(&registered, &declared).is_ok());
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

#[test]
fn schema_for_rpc_method_finds_known_method() {
    let schema = schema_for_rpc_method("openhuman.health_snapshot");
    assert!(schema.is_some(), "health.snapshot should be findable");
    let s = schema.unwrap();
    assert_eq!(s.namespace, "health");
    assert_eq!(s.function, "snapshot");
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

// --- validate_registry edge cases ---------------------------------------

#[test]
fn validate_registry_rejects_empty_namespace() {
    let declared = vec![schema("", "fn", vec![])];
    let registered = vec![RegisteredController {
        schema: declared[0].clone(),
        handler: noop_handler,
    }];
    let err = validate_registry(&registered, &declared).unwrap_err();
    assert!(err.contains("namespace must not be empty"));
}

#[test]
fn validate_registry_rejects_empty_function() {
    let declared = vec![schema("ns", "", vec![])];
    let registered = vec![RegisteredController {
        schema: declared[0].clone(),
        handler: noop_handler,
    }];
    let err = validate_registry(&registered, &declared).unwrap_err();
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
    let err = validate_registry(&registered, &declared).unwrap_err();
    assert!(err.contains("namespace must not be empty"));
}

#[test]
fn validate_registry_rejects_declared_without_registered() {
    let declared = vec![schema("a", "b", vec![])];
    let registered: Vec<RegisteredController> = vec![];
    let err = validate_registry(&registered, &declared).unwrap_err();
    assert!(err.contains("declared controller `a.b` has no registered handler"));
}

#[test]
fn validate_registry_rejects_registered_without_declared() {
    let declared: Vec<ControllerSchema> = vec![];
    let registered = vec![RegisteredController {
        schema: schema("a", "b", vec![]),
        handler: noop_handler,
    }];
    let err = validate_registry(&registered, &declared).unwrap_err();
    assert!(err.contains("registered controller `a.b` has no declared schema"));
}

#[test]
fn validate_registry_rejects_duplicate_registered_controllers() {
    let s = schema("a", "b", vec![]);
    let declared = vec![s.clone()];
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
    let err = validate_registry(&registered, &declared).unwrap_err();
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
