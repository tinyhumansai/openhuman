//! Schemas and handlers for the tool-scoped memory RPC methods.
//!
//! Six methods are exposed under the `memory` namespace:
//!
//! - `tool_rule_put`           — upsert a rule
//! - `tool_rule_get`           — fetch a single rule by id
//! - `tool_rule_list`          — list every rule for a tool
//! - `tool_rule_delete`        — delete a rule
//! - `tool_rules_for_prompt`   — pre-rendered prompt block + structured rules
//! - `tool_rules_json`         — raw JSON list for envelope consumers

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::rpc::{
    self, ToolRuleListParams, ToolRulePutParams, ToolRuleRefParams, ToolRulesForPromptParams,
};

use super::{parse_params, to_json};

pub(super) const FUNCTIONS: &[&str] = &[
    "tool_rule_put",
    "tool_rule_get",
    "tool_rule_list",
    "tool_rule_delete",
    "tool_rules_for_prompt",
    "tool_rules_json",
];

pub(super) fn controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("tool_rule_put").unwrap(),
            handler: handle_tool_rule_put,
        },
        RegisteredController {
            schema: schema("tool_rule_get").unwrap(),
            handler: handle_tool_rule_get,
        },
        RegisteredController {
            schema: schema("tool_rule_list").unwrap(),
            handler: handle_tool_rule_list,
        },
        RegisteredController {
            schema: schema("tool_rule_delete").unwrap(),
            handler: handle_tool_rule_delete,
        },
        RegisteredController {
            schema: schema("tool_rules_for_prompt").unwrap(),
            handler: handle_tool_rules_for_prompt,
        },
        RegisteredController {
            schema: schema("tool_rules_json").unwrap(),
            handler: handle_tool_rules_json,
        },
    ]
}

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    Some(match function {
        "tool_rule_put" => ControllerSchema {
            namespace: "memory",
            function: "tool_rule_put",
            description:
                "Upsert a tool-scoped memory rule. Stored in namespace `tool-{tool_name}`, separate \
                from generic `global` / `skill-{id}` / `tool_effectiveness` namespaces. Use \
                `priority='critical'` for safety-critical rules that must survive context compression.",
            inputs: vec![
                FieldSchema {
                    name: "tool_name",
                    ty: TypeSchema::String,
                    comment: "Tool the rule applies to.",
                    required: true,
                },
                FieldSchema {
                    name: "rule",
                    ty: TypeSchema::String,
                    comment: "Natural-language rule body.",
                    required: true,
                },
                FieldSchema {
                    name: "priority",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Priority — 'critical', 'high', or 'normal'. Defaults to 'normal'.",
                    required: false,
                },
                FieldSchema {
                    name: "source",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "Provenance — 'user_explicit', 'post_turn', or 'programmatic'. Defaults to \
                        'programmatic'.",
                    required: false,
                },
                FieldSchema {
                    name: "tags",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Optional tags for filtering.",
                    required: false,
                },
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional rule id — supplied to upsert in place.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Stored rule, including assigned id and timestamps.",
                required: true,
            }],
        },
        "tool_rule_get" => ControllerSchema {
            namespace: "memory",
            function: "tool_rule_get",
            description: "Fetch a tool-scoped rule by `(tool_name, id)`. Returns null when missing.",
            inputs: vec![
                FieldSchema {
                    name: "tool_name",
                    ty: TypeSchema::String,
                    comment: "Tool the rule applies to.",
                    required: true,
                },
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "Rule id.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Stored rule or null.",
                required: true,
            }],
        },
        "tool_rule_list" => ControllerSchema {
            namespace: "memory",
            function: "tool_rule_list",
            description:
                "List every rule for a tool, sorted by priority (critical → high → normal) and then \
                by `updated_at` descending.",
            inputs: vec![FieldSchema {
                name: "tool_name",
                ty: TypeSchema::String,
                comment: "Tool to list rules for.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of stored rules.",
                required: true,
            }],
        },
        "tool_rule_delete" => ControllerSchema {
            namespace: "memory",
            function: "tool_rule_delete",
            description:
                "Delete a tool-scoped rule. Returns true if the rule existed before deletion.",
            inputs: vec![
                FieldSchema {
                    name: "tool_name",
                    ty: TypeSchema::String,
                    comment: "Tool the rule applies to.",
                    required: true,
                },
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "Rule id.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Bool,
                comment: "True when the rule existed.",
                required: true,
            }],
        },
        "tool_rules_for_prompt" => ControllerSchema {
            namespace: "memory",
            function: "tool_rules_for_prompt",
            description:
                "Pre-fetch Critical + High priority rules for prompt injection. Returns the \
                rendered Markdown block (ready for the system prompt) plus the structured rule \
                snapshot. Used by the session builder to surface rules during tool-selection and \
                pre-execution phases.",
            inputs: vec![FieldSchema {
                name: "tools",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Constrain to these tools. Empty means scan every known tool namespace.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{ rendered: string, rules: ToolMemoryRule[] }.",
                required: true,
            }],
        },
        "tool_rules_json" => ControllerSchema {
            namespace: "memory",
            function: "tool_rules_json",
            description:
                "Return the full rule list for a tool as raw JSON — useful for envelope consumers.",
            inputs: vec![FieldSchema {
                name: "tool_name",
                ty: TypeSchema::String,
                comment: "Tool to list rules for.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of stored rules as JSON.",
                required: true,
            }],
        },
        _ => return None,
    })
}

fn handle_tool_rule_put(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ToolRulePutParams>(params)?;
        to_json(rpc::tool_rule_put(payload).await?)
    })
}

fn handle_tool_rule_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ToolRuleRefParams>(params)?;
        to_json(rpc::tool_rule_get(payload).await?)
    })
}

fn handle_tool_rule_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ToolRuleListParams>(params)?;
        to_json(rpc::tool_rule_list(payload).await?)
    })
}

fn handle_tool_rule_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ToolRuleRefParams>(params)?;
        to_json(rpc::tool_rule_delete(payload).await?)
    })
}

fn handle_tool_rules_for_prompt(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ToolRulesForPromptParams>(params)?;
        to_json(rpc::tool_rules_for_prompt(payload).await?)
    })
}

fn handle_tool_rules_json(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<ToolRuleListParams>(params)?;
        to_json(rpc::tool_rules_json(payload).await?)
    })
}
