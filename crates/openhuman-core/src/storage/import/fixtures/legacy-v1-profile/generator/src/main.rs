use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tinystoragedrivers::secrets::{DerivedKeys, DocumentSecrets, SecretStore};
use tinystoragedrivers::{CollectionSpec, Precondition, Scope, StorageConfig, WriteOptions};
use zeroize::Zeroizing;

const PROFILE_ID: &str = "u1";
const CREATED_AT_MS: i64 = 1_738_548_306_000;

// These wire types are copied from the historical owner models at
// OpenHuman 2d3e22636e^ (paths listed in the fixture README). Keeping this
// generator standalone avoids linking the current host or its patched module
// graph into a historical TSD v1 writer. The fixture test decodes the output
// with today's owner models after the owner's schema step runs.
#[derive(Serialize)]
struct LegacyCostRecord {
    id: String,
    usage: LegacyTokenUsage,
    session_id: String,
}

#[derive(Serialize)]
struct LegacyTokenUsage {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cached_input_tokens: u64,
    cache_creation_tokens: u64,
    reasoning_tokens: u64,
    cost_usd: f64,
    cost_source: LegacyCostSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_run_id: Option<String>,
    #[serde(skip_serializing_if = "LegacyUsageScope::is_empty")]
    scope: LegacyUsageScope,
    timestamp: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum LegacyCostSource {
    Estimated,
    ProviderCharged,
    Unknown,
}

#[derive(Default, Serialize, Deserialize)]
struct LegacyUsageScope {}

impl LegacyUsageScope {
    fn is_empty(&self) -> bool {
        true
    }
}

#[derive(Serialize)]
struct LegacyArtifactMeta {
    id: String,
    kind: LegacyArtifactKind,
    title: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_root: Option<String>,
    size_bytes: u64,
    status: LegacyArtifactStatus,
    created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum LegacyArtifactKind {
    Presentation,
    Document,
    Image,
    Video,
    Other,
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum LegacyArtifactStatus {
    Pending,
    Ready,
    Failed,
}

#[derive(Serialize)]
struct LegacyProfileMeta {
    profile_id: String,
    created_at: u64,
    layout_version: u32,
}

/// The v1 `approvals` collection payload consumed by the approval schema step.
#[derive(Serialize)]
struct LegacyApprovalRequest {
    tool_name: String,
    action_summary: String,
    args_redacted: String,
    session_id: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    pending: bool,
}

/// The v1 TinyFlows catalog row consumed by its `flows_definitions` step.
#[derive(Serialize)]
struct LegacyFlowDefinition {
    name: String,
    graph_json: String,
    enabled: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    require_approval: bool,
    incarnation: String,
}

/// The transcript-key payload in `threads_transcript_keys`.
#[derive(Serialize)]
struct LegacyTranscriptKey {
    key: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: legacy-v1-profile-writer <output-storage.db>")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let url = format!("sqlite:{}", path.display());
    let backend = tinystoragedrivers::open(&StorageConfig::parse(&url)?).await?;

    // These transformations are the pre-schema-v2 owner writers from
    // OpenHuman commit 2d3e22636e^: cost records retained their typed body and
    // timestamp, artifact documents stored created_ms, args were wrapped,
    // and the profile registry kept created_at in document metadata. The
    // actual disk layout and writes are supplied by the historical public
    // TSD v1 API pinned in this package.
    let user_scope = backend.for_scope(&Scope::local())?;
    let docs = user_scope.documents();

    let cost = LegacyCostRecord {
        id: "cost-0001".into(),
        session_id: "session-fixture".into(),
        usage: LegacyTokenUsage {
            model: "fixture-model".into(),
            input_tokens: 12,
            output_tokens: 3,
            total_tokens: 15,
            cached_input_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cost_usd: 0.000_015,
            cost_source: LegacyCostSource::Estimated,
            run_id: None,
            root_run_id: None,
            scope: LegacyUsageScope::default(),
            timestamp: DateTime::parse_from_rfc3339("2025-02-03T04:05:06.789+02:00")?
                .with_timezone(&Utc),
        },
    };
    let mut cost_body = serde_json::to_value(cost)?;
    cost_body["ts_ms"] = 1_738_548_306_789_i64.into();
    put(docs, "cost_records", "cost-0001", cost_body).await?;

    let artifact = LegacyArtifactMeta {
        id: "artifact-1".into(),
        kind: LegacyArtifactKind::Document,
        title: "Fixture plan".into(),
        path: "artifact-1/plan.md".into(),
        file: None,
        file_root: None,
        size_bytes: 4,
        status: LegacyArtifactStatus::Ready,
        created_at: DateTime::parse_from_rfc3339("2025-02-03T04:05:06Z")?.with_timezone(&Utc),
        error: None,
        thread_id: Some("thread-1".into()),
        tool_call_id: None,
    };
    let mut artifact_body = serde_json::to_value(artifact)?;
    let created_at = artifact_body
        .as_object_mut()
        .and_then(|fields| fields.remove("created_at"))
        .ok_or("ArtifactMeta serialization omitted created_at")?;
    artifact_body["created_ms"] = created_at
        .as_str()
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|at| at.timestamp_millis())
        .ok_or("ArtifactMeta created_at was not RFC3339")?
        .into();
    put(docs, "artifacts", "artifact-1", artifact_body).await?;
    put(
        docs,
        "artifact_args",
        "artifact-1",
        json!({"args": {"title": "Fixture plan", "format": "markdown"}}),
    )
    .await?;

    let cluster = backend.for_scope(&Scope::new("cluster")?)?;
    let profile = LegacyProfileMeta {
        profile_id: PROFILE_ID.into(),
        created_at: u64::try_from(CREATED_AT_MS / 1000)?,
        layout_version: 1,
    };
    let mut profile_body = serde_json::to_value(profile)?;
    profile_body
        .as_object_mut()
        .ok_or("ProfileMeta did not serialize as an object")?
        .remove("created_at");
    let docs = cluster.documents();
    docs.ensure_collection(&CollectionSpec::new("profiles"))
        .await?;
    docs.put_with(
        "profiles",
        PROFILE_ID,
        profile_body,
        Precondition::Absent,
        WriteOptions::new().created_ms(CREATED_AT_MS),
    )
    .await?;

    // Records in an existing desktop profile database use the historical
    // local scope: the profile directory itself supplied user isolation.
    // Their owners must continue to read them after the real v1-to-v2 boot
    // migration.
    // This is the approvals v1 owner document from the historic schema:
    // approvals.requests did not exist yet. The approvals owner's v2 schema
    // migration converts these RFC3339 timestamps and collection name.
    let approval = LegacyApprovalRequest {
        tool_name: "shell".into(),
        action_summary: "Fixture approval".into(),
        args_redacted: "{}".into(),
        session_id: "fixture-session".into(),
        created_at: DateTime::parse_from_rfc3339("2025-02-03T04:05:06Z")?.with_timezone(&Utc),
        expires_at: DateTime::parse_from_rfc3339("2099-01-01T00:00:00Z")?.with_timezone(&Utc),
        pending: true,
    };
    put(
        user_scope.documents(),
        "approvals",
        "fixture-approval",
        serde_json::to_value(approval)?,
    )
    .await?;
    let thread_id = "fixture-thread";
    let thread_digest = "d033a44fe04c14424be31d765b05f363fddeef939017517f69cf2914980ae87f";
    let incarnation = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
    put(
        user_scope.documents(),
        "conversation_threads_v1",
        thread_digest,
        json!({
            "schema": 1,
            "incarnation": incarnation,
            "deleted": false,
            "visible_end": 1,
            "thread": {
                "id": thread_id,
                "title": "Fixture thread",
                "isActive": true,
                "messageCount": 1,
                "lastMessageAt": "2025-02-04T04:05:07Z",
                "createdAt": "2025-02-04T04:05:06Z",
                "labels": ["history"]
            }
        }),
    )
    .await?;
    user_scope
        .streams()
        .append(
            &format!("conversation_messages_v1:{thread_digest}:{incarnation}"),
            json!({
                "schema": 1,
                "incarnation": incarnation,
                "message": {
                    "id": "fixture-message",
                    "content": "historical profile message",
                    "type": "text",
                    "extraMetadata": null,
                    "sender": "user",
                    "createdAt": "2025-02-04T04:05:07Z"
                }
            }),
        )
        .await?;
    // The thread owner historically keyed transcript rows by the raw thread
    // id in `threads_transcript_keys`; its schema step rekeys this to the
    // current owner collection and escaped id.
    put(
        user_scope.documents(),
        "threads_transcript_keys",
        thread_id,
        serde_json::to_value(LegacyTranscriptKey {
            key: "fixture-transcript-key".into(),
        })?,
    )
    .await?;

    let legacy_flow_scope = backend.for_scope(&Scope::local())?;
    let flow = LegacyFlowDefinition {
        name: "Fixture flow".into(),
        graph_json: "{\"nodes\":[{\"id\":\"trigger\",\"kind\":\"trigger\",\"name\":\"Manual\"}],\"edges\":[]}".into(),
        enabled: false,
        created_at: DateTime::parse_from_rfc3339("2025-02-03T04:05:06Z")?.with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339("2025-02-03T04:05:06Z")?.with_timezone(&Utc),
        require_approval: false,
        incarnation: "fixture-incarnation".into(),
    };
    put(
        legacy_flow_scope.documents(),
        "flows_definitions",
        "fixture-flow",
        serde_json::to_value(flow)?,
    )
    .await?;

    let secret_store = DocumentSecrets::new(
        &user_scope,
        Arc::new(DerivedKeys::new(Zeroizing::new([0x5a; 32]))),
    );
    secret_store
        .set("fixture-secret", b"historical-secret-value")
        .await?;

    backend
        .checkpoint(tinystoragedrivers::Checkpoint::Truncate)
        .await?;
    println!("written {}", path.display());
    Ok(())
}

async fn put(
    docs: &std::sync::Arc<dyn tinystoragedrivers::DocumentStore>,
    collection: &str,
    id: &str,
    body: Value,
) -> Result<(), tinystoragedrivers::StorageError> {
    docs.ensure_collection(&CollectionSpec::new(collection))
        .await?;
    docs.put(collection, id, body, Precondition::Absent).await?;
    Ok(())
}
