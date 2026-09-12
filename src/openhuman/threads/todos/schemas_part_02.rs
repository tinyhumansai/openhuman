fn handle_reclaim_stale(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ReclaimStaleParams>(params)?;
        let loc = thread_location(&p.thread_id).await?;
        let limits = runs::RunLimits {
            heartbeat_stale_secs: p
                .heartbeat_stale_secs
                .unwrap_or(runs::DEFAULT_HEARTBEAT_STALE_SECS),
            claim_ttl_secs: p.claim_ttl_secs.unwrap_or(runs::DEFAULT_CLAIM_TTL_SECS),
            max_reclaim_count: p
                .max_reclaim_count
                .unwrap_or(runs::DEFAULT_MAX_RECLAIM_COUNT),
        };
        tracing::debug!(
            thread_id = %p.thread_id,
            ?limits,
            "[rpc][todos] reclaim_stale entry"
        );
        let result = runs::reclaim_stale(&loc, &limits).await?;
        serde_json::to_value(&result).map_err(|e| format!("serialize reclaim result: {e}"))
    })
}

// ── helpers ──────────────────────────────────────────────────────────

async fn thread_location(thread_id: &str) -> Result<BoardLocation, String> {
    let trimmed = thread_id.trim();
    if trimmed.is_empty() {
        return Err("thread_id must not be empty".to_string());
    }
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    Ok(BoardLocation::Thread {
        workspace_dir: config.workspace_dir,
        thread_id: trimmed.to_string(),
    })
}

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn snapshot_to_json(snap: TodosSnapshot) -> Result<Value, String> {
    serde_json::to_value(&snap).map_err(|e| format!("serialize snapshot: {e}"))
}

fn thread_id_input() -> FieldSchema {
    FieldSchema {
        name: "thread_id",
        ty: TypeSchema::String,
        comment: "Conversation thread identifier (same id used by `threads.task_board_*`).",
        required: true,
    }
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn string_array_input(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
        comment,
        required: false,
    }
}

/// The `cards` input of `todos.replace`, spelled out field by field.
///
/// This was a bare `TypeSchema::Json` commented "Array of card objects (id may
/// be empty — server generates)", which is not enough to construct one and was
/// actively misleading in two ways (#6087):
///
///  * `handle_replace` deserializes each entry into `TaskBoardCard`, whose
///    `id`, `title` and `status` carry **no** `#[serde(default)]` — so all
///    three keys are mandatory. "id may be empty" is true of the *string* and
///    false of the *key*: omitting it fails with ``missing field `id` ``.
///  * the text field is `title`, while the sibling `todos.add` / `todos.edit`
///    inputs in this same namespace call it `content`. A caller who reached for
///    the namespace's own vocabulary got ``missing field `title` ``.
///
/// Names below are the wire names: `TaskBoardCard` is
/// `#[serde(rename_all = "camelCase")]` and `TaskCardStatus` is
/// `#[serde(rename_all = "snake_case")]`, so the declaration must use
/// `assignedAgent` (not `assigned_agent`) and `in_progress` (not `inProgress`).
/// Every optional field carries `#[serde(default)]` upstream, so the optional
/// markings here are load-bearing rather than decorative.
fn replace_cards_input() -> FieldSchema {
    FieldSchema {
        name: "cards",
        ty: TypeSchema::Array(Box::new(TypeSchema::Object {
            fields: vec![
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "Stable card id (`task-<n>`). The KEY is required; \
                              pass an empty string to have the server generate one.",
                    required: true,
                },
                FieldSchema {
                    name: "title",
                    ty: TypeSchema::String,
                    comment: "One-line title. NOTE: `todos.add` / `todos.edit` \
                              call this same field `content`; here it is `title`.",
                    required: true,
                },
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::Enum {
                        variants: vec![
                            "todo",
                            "awaiting_approval",
                            "ready",
                            "in_progress",
                            "blocked",
                            "done",
                            "rejected",
                        ],
                    },
                    comment: "Lifecycle state. At most one card may be `in_progress`.",
                    required: true,
                },
                optional_string("objective", "Richer objective for the card."),
                defaulted_field(
                    "plan",
                    TypeSchema::Array(Box::new(TypeSchema::String)),
                    "Ordered plan steps. Omit the key to default to []; `null` is rejected.",
                ),
                optional_string("assignedAgent", "Agent assigned to run this card."),
                defaulted_field(
                    "allowedTools",
                    TypeSchema::Array(Box::new(TypeSchema::String)),
                    "Tools the assigned agent may use. Omit to default to []; `null` is rejected.",
                ),
                FieldSchema {
                    name: "approvalMode",
                    // `TaskApprovalMode` is a two-variant enum, not a free
                    // string: declaring `Option(String)` advertised every
                    // string as valid, so a catalog-conforming `"sometimes"`
                    // would come back `invalid params`. Wrapped in `Option`
                    // because the field really is `Option<TaskApprovalMode>`
                    // upstream, so `null` genuinely is accepted here — unlike
                    // the `defaulted_field` group above.
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["required", "not_required"],
                    })),
                    comment: "Plan-approval mode, when the card is gated. \
                              `null` clears it.",
                    required: false,
                },
                defaulted_field(
                    "acceptanceCriteria",
                    TypeSchema::Array(Box::new(TypeSchema::String)),
                    "Acceptance criteria that define \"done\". Omit to default to []; `null` is rejected.",
                ),
                defaulted_field(
                    "evidence",
                    TypeSchema::Array(Box::new(TypeSchema::String)),
                    "Evidence gathered toward completion. Omit to default to []; `null` is rejected.",
                ),
                optional_string("notes", "Free-form notes."),
                optional_string("blocker", "Reason, when `status == blocked`."),
                optional_string(
                    "sessionThreadId",
                    "Thread the card's own agent session runs in.",
                ),
                FieldSchema {
                    name: "sourceMetadata",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Provenance blob carried through untouched.",
                    required: false,
                },
                defaulted_field(
                    "order",
                    // `TaskBoardCard::order` is a `u32`, but `TypeSchema` has
                    // no narrower unsigned type and no bounds, so `U64` is the
                    // closest available declaration and the ceiling has to be
                    // stated in prose. A value above `u32::MAX` passes schema
                    // validation and is then refused by the handler's
                    // deserialization. That imprecision is shared by every
                    // `TypeSchema::U64` declaration in the catalog (200-odd of
                    // them), so closing it means adding a bounded integer to
                    // `core::TypeSchema` rather than editing this one field —
                    // tracked as #6137. Drop this caveat when that lands.
                    TypeSchema::U64,
                    "Sort position, 0..=4294967295 (u32). Omit to default to 0; \
                     `null` is rejected, and a value above the u32 ceiling is \
                     refused by the handler rather than by schema validation.",
                ),
                defaulted_field(
                    "updatedAt",
                    TypeSchema::String,
                    "Last-update stamp, server-maintained. Omit it; `null` is rejected.",
                ),
            ],
        })),
        comment: "Full replacement list. Each entry MUST carry `id`, `title` and \
                  `status`; every other field is optional. Note `title`, not \
                  `content` — see the field comments.",
        required: true,
    }
}

/// A `todos.replace` card field that upstream `#[serde(default)]`s.
///
/// Declared with its **non-`Option`** type and `required: false`, which is the
/// accurate statement of the contract: the key may be *omitted* (serde supplies
/// the default) but may not be sent as `null`. `TaskBoardCard`'s `plan`,
/// `allowedTools`, `acceptanceCriteria` and `evidence` are `Vec<String>`,
/// `order` is `u32` and `updatedAt` is `String` — none is an `Option`, so
/// `null` fails deserialization with `invalid type: null`.
///
/// Declaring these as `Option(...)` would advertise `null` as valid and put the
/// catalog right back to describing a call the handler rejects, which is the
/// defect #6087 exists to remove.
fn defaulted_field(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: false,
    }
}

fn snapshot_output() -> FieldSchema {
    FieldSchema {
        name: "snapshot",
        ty: TypeSchema::Json,
        comment: "Object with `threadId`, `cards`, and a `markdown` rendering of the list.",
        required: true,
    }
}
