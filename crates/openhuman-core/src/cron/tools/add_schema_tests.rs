use super::*;
use serde_json::json;

fn expected_cron_add() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Short job name, e.g. 'drink_water'."
            },
            "schedule": {
                "oneOf": [
                    {
                        "type": "object",
                        "description": "Runs once.",
                        "properties": {
                            "kind": {
                                "type": "string",
                                "const": "at"
                            },
                            "at": {
                                "type": "string",
                                "description": "RFC 3339 UTC instant."
                            }
                        },
                        "required": [
                            "kind",
                            "at"
                        ],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "description": "Repeats on a cron expression.",
                        "properties": {
                            "kind": {
                                "type": "string",
                                "const": "cron"
                            },
                            "expr": {
                                "type": "string"
                            },
                            "tz": {
                                "type": "string",
                                "description": "IANA zone."
                            },
                            "active_hours": {
                                "type": "object",
                                "properties": {
                                    "start": {
                                        "type": "string"
                                    },
                                    "end": {
                                        "type": "string"
                                    }
                                },
                                "required": [
                                    "start",
                                    "end"
                                ],
                                "additionalProperties": false
                            }
                        },
                        "required": [
                            "kind",
                            "expr"
                        ],
                        "additionalProperties": false
                    },
                    {
                        "type": "object",
                        "description": "Repeats every N ms.",
                        "properties": {
                            "kind": {
                                "type": "string",
                                "const": "every"
                            },
                            "every_ms": {
                                "type": "integer"
                            }
                        },
                        "required": [
                            "kind",
                            "every_ms"
                        ],
                        "additionalProperties": false
                    }
                ]
            },
            "job_type": {
                "type": "string",
                "enum": [
                    "agent",
                    "shell"
                ]
            },
            "prompt": {
                "type": "string",
                "description": "Agent job: what the run delivers."
            },
            "command": {
                "type": "string",
                "description": "Shell job."
            },
            "session_target": {
                "type": "string",
                "enum": [
                    "isolated",
                    "current",
                    "main"
                ]
            },
            "model": {
                "type": "string"
            },
            "delivery": {
                "type": "object",
                "description": "Omit to reply where created.",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": [
                            "origin",
                            "proactive",
                            "announce",
                            "none"
                        ]
                    },
                    "channel": {
                        "type": "string"
                    },
                    "to": {
                        "type": "string"
                    },
                    "best_effort": {
                        "type": "boolean"
                    }
                }
            },
            "delete_after_run": {
                "type": "boolean"
            }
        },
        "required": [
            "name",
            "schedule"
        ]
    })
}

#[test]
fn cron_add_static_schema_matches_json_literal() {
    let tool = CronAddTool::new(
        std::sync::Arc::new(crate::config::Config::default()),
        std::sync::Arc::new(crate::security::SecurityPolicy::default()),
    );
    assert_eq!(
        tinytools::Tool::parameters_schema(&tool),
        expected_cron_add()
    );
}
