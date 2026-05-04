use super::*;

#[test]
fn test_remove_unsupported_keywords() {
    let schema = json!({
        "type": "string",
        "minLength": 1,
        "maxLength": 100,
        "pattern": "^[a-z]+$",
        "description": "A lowercase string"
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert_eq!(cleaned["type"], "string");
    assert_eq!(cleaned["description"], "A lowercase string");
    assert!(cleaned.get("minLength").is_none());
    assert!(cleaned.get("maxLength").is_none());
    assert!(cleaned.get("pattern").is_none());
}

#[test]
fn test_resolve_ref() {
    let schema = json!({
        "type": "object",
        "properties": {
            "age": {
                "$ref": "#/$defs/Age"
            }
        },
        "$defs": {
            "Age": {
                "type": "integer",
                "minimum": 0
            }
        }
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert_eq!(cleaned["properties"]["age"]["type"], "integer");
    assert!(cleaned["properties"]["age"].get("minimum").is_none()); // Stripped by Gemini strategy
    assert!(cleaned.get("$defs").is_none());
}

#[test]
fn test_flatten_literal_union() {
    let schema = json!({
        "anyOf": [
            { "const": "admin", "type": "string" },
            { "const": "user", "type": "string" },
            { "const": "guest", "type": "string" }
        ]
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert_eq!(cleaned["type"], "string");
    assert!(cleaned["enum"].is_array());
    let enum_values = cleaned["enum"].as_array().unwrap();
    assert_eq!(enum_values.len(), 3);
    assert!(enum_values.contains(&json!("admin")));
    assert!(enum_values.contains(&json!("user")));
    assert!(enum_values.contains(&json!("guest")));
}

#[test]
fn test_strip_null_from_union() {
    let schema = json!({
        "oneOf": [
            { "type": "string" },
            { "type": "null" }
        ]
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    // Should simplify to just { type: "string" }
    assert_eq!(cleaned["type"], "string");
    assert!(cleaned.get("oneOf").is_none());
}

#[test]
fn test_const_to_enum() {
    let schema = json!({
        "const": "fixed_value",
        "description": "A constant"
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert_eq!(cleaned["enum"], json!(["fixed_value"]));
    assert_eq!(cleaned["description"], "A constant");
    assert!(cleaned.get("const").is_none());
}

#[test]
fn test_preserve_metadata() {
    let schema = json!({
        "$ref": "#/$defs/Name",
        "description": "User's name",
        "title": "Name Field",
        "default": "Anonymous",
        "$defs": {
            "Name": {
                "type": "string"
            }
        }
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert_eq!(cleaned["type"], "string");
    assert_eq!(cleaned["description"], "User's name");
    assert_eq!(cleaned["title"], "Name Field");
    assert_eq!(cleaned["default"], "Anonymous");
}

#[test]
fn test_circular_ref_prevention() {
    let schema = json!({
        "type": "object",
        "properties": {
            "parent": {
                "$ref": "#/$defs/Node"
            }
        },
        "$defs": {
            "Node": {
                "type": "object",
                "properties": {
                    "child": {
                        "$ref": "#/$defs/Node"
                    }
                }
            }
        }
    });

    // Should not panic on circular reference
    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert_eq!(cleaned["properties"]["parent"]["type"], "object");
    // Circular reference should be broken
}

#[test]
fn test_validate_schema() {
    let valid = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        }
    });

    assert!(SchemaCleanr::validate(&valid).is_ok());

    let invalid = json!({
        "properties": {
            "name": { "type": "string" }
        }
    });

    assert!(SchemaCleanr::validate(&invalid).is_err());
}

#[test]
fn test_strategy_differences() {
    let schema = json!({
        "type": "string",
        "minLength": 1,
        "description": "A string field"
    });

    // Gemini: Most restrictive (removes minLength)
    let gemini = SchemaCleanr::clean_for_gemini(schema.clone());
    assert!(gemini.get("minLength").is_none());
    assert_eq!(gemini["type"], "string");
    assert_eq!(gemini["description"], "A string field");

    // OpenAI: Most permissive (keeps minLength)
    let openai = SchemaCleanr::clean_for_openai(schema.clone());
    assert_eq!(openai["minLength"], 1); // OpenAI allows validation keywords
    assert_eq!(openai["type"], "string");
}

#[test]
fn test_nested_properties() {
    let schema = json!({
        "type": "object",
        "properties": {
            "user": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "minLength": 1
                    }
                },
                "additionalProperties": false
            }
        }
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert!(cleaned["properties"]["user"]["properties"]["name"]
        .get("minLength")
        .is_none());
    assert!(cleaned["properties"]["user"]
        .get("additionalProperties")
        .is_none());
}

#[test]
fn test_type_array_null_removal() {
    let schema = json!({
        "type": ["string", "null"]
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    // Should simplify to just "string"
    assert_eq!(cleaned["type"], "string");
}

#[test]
fn test_type_array_only_null_preserved() {
    let schema = json!({
        "type": ["null"]
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert_eq!(cleaned["type"], "null");
}

#[test]
fn test_ref_with_json_pointer_escape() {
    let schema = json!({
        "$ref": "#/$defs/Foo~1Bar",
        "$defs": {
            "Foo/Bar": {
                "type": "string"
            }
        }
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert_eq!(cleaned["type"], "string");
}

#[test]
fn test_skip_type_when_non_simplifiable_union_exists() {
    let schema = json!({
        "type": "object",
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "a": { "type": "string" }
                }
            },
            {
                "type": "object",
                "properties": {
                    "b": { "type": "number" }
                }
            }
        ]
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert!(cleaned.get("type").is_none());
    assert!(cleaned.get("oneOf").is_some());
}

#[test]
fn test_clean_nested_unknown_schema_keyword() {
    let schema = json!({
        "not": {
            "$ref": "#/$defs/Age"
        },
        "$defs": {
            "Age": {
                "type": "integer",
                "minimum": 0
            }
        }
    });

    let cleaned = SchemaCleanr::clean_for_gemini(schema);

    assert_eq!(cleaned["not"]["type"], "integer");
    assert!(cleaned["not"].get("minimum").is_none());
}
