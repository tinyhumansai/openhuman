//! The canonical ingest payload shapes — chat, email, document — exactly as
//! they came home from `tinycortex::memory::ingest::canonicalize` (#5560),
//! plus the `invalid_payload_message` formatter their handlers share.
//!
//! Split out of `rpc_part_01.rs` purely for the file-layout gate; every name
//! is re-exported from there, so no import path changed. The serde behaviour
//! in here is wire contract: the timestamp leniency, the provider default,
//! and the epoch-ms/RFC-3339 dual parse are what deployed producers already
//! send.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tinymemory_api::chunks::SourceKind;

// ── The `memory_tree_ingest` payload shapes (#5560) ──────────────────────────
//
// `ChatBatch`, `EmailThread` and `DocumentInput` were imported from
// `tinycortex::memory::ingest::canonicalize`. They are defined here now, and
// the reason is that **this host is their only reader**: they are the request
// half of an OpenHuman JSON-RPC method, deserialised by [`ingest_rpc`] below
// and turned into contract [`IngestItem`]s by the three mappers further down.
// Nothing on this side ever hands one of these structs to the engine — the
// items are what cross the bus, and the driver rebuilds its own copy of these
// shapes on the far end from those items. Two independent readers of one JSON
// wire format, which is the same arrangement `memory::rpc_models`,
// `memory::safety` and `memory::source_scope` already landed on.
//
// So there is no contract door to route this at, and asking for one would be
// asking `tinymemory-api` to carry a payload that never crosses the bus. What
// pins the *shape* is the wire, not the type: the driver's reconstruction is
// documented field-for-field on [`email_items`] and [`chat_items`], and
// `rpc_tests_part_01_tests` pins the serde tolerances. Change a field name
// here and the round trip breaks in exactly the way those tests describe —
// which is the same exposure the import had, since a rename upstream would
// have reshaped this RPC's published request body without anything here
// failing to compile.
//
// `Serialize` is derived alongside `Deserialize` because the engine derived
// both and the round-trip is what the tests assert; nothing in `src/`
// serialises one.

/// Serde default for the two chat/mail message timestamps.
///
/// A payload missing the field falls back to `now()` rather than rejecting the
/// whole batch, so a client with version skew (or a third-party integration
/// that never sent one) does not lose an entire thread to one absent key.
fn ingest_timestamp_now() -> DateTime<Utc> {
    Utc::now()
}

/// Serde default for [`DocumentInput::provider`].
fn default_document_provider() -> String {
    "unknown".to_string()
}

/// Accept a timestamp as epoch-milliseconds, an RFC 3339 / ISO-8601 string, or
/// `null`.
///
/// Three shapes because three generations of clients send three shapes, and
/// the alternative to accepting all of them is silently dropping whichever the
/// caller happens to use.
///
/// The near-epoch rejection is the load-bearing part: contemporary epoch
/// *seconds* are ten digits and epoch *milliseconds* are thirteen, so a
/// seconds value passed here would decode to 1970 and quietly poison ordering
/// and staleness. Rejecting the ambiguous range makes that a loud error at the
/// seam instead.
fn deserialize_flexible_timestamp<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawTs {
        Millis(i64),
        Text(String),
        Null,
    }

    fn epoch_millis<E: serde::de::Error>(ms: i64) -> Result<DateTime<Utc>, E> {
        const MIN_PLAUSIBLE_EPOCH_MILLIS: u64 = 100_000_000_000;
        if ms.unsigned_abs() < MIN_PLAUSIBLE_EPOCH_MILLIS {
            return Err(E::custom(format!(
                "epoch-ms value {ms} is too small; pass milliseconds, not seconds"
            )));
        }
        chrono::TimeZone::timestamp_millis_opt(&Utc, ms)
            .single()
            .ok_or_else(|| E::custom(format!("invalid epoch-ms: {ms}")))
    }

    let raw = RawTs::deserialize(deserializer)?;
    match raw {
        RawTs::Null => Ok(Utc::now()),
        RawTs::Millis(ms) => epoch_millis(ms),
        RawTs::Text(s) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
                return Ok(dt.with_timezone(&Utc));
            }
            if let Ok(ms) = s.parse::<i64>() {
                return epoch_millis(ms);
            }
            Err(serde::de::Error::custom(format!(
                "cannot parse '{s}' as RFC 3339 or epoch-ms"
            )))
        }
    }
}

/// One chat message in a channel/group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Author display name or id.
    pub author: String,
    /// When the message was sent (epoch-ms integer or RFC 3339 string).
    /// When absent from the payload, defaults to `Utc::now()` — see
    /// [`ingest_timestamp_now`].
    #[serde(
        default = "ingest_timestamp_now",
        serialize_with = "chrono::serde::ts_milliseconds::serialize",
        deserialize_with = "deserialize_flexible_timestamp"
    )]
    pub timestamp: DateTime<Utc>,
    /// Plain text / markdown body.
    pub text: String,
    /// Optional per-message provenance pointer (permalink or `platform://...`).
    #[serde(default)]
    pub source_ref: Option<String>,
}

/// Adapter input — a batch of messages from one logical channel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatBatch {
    /// Platform name (e.g. `slack`, `discord`, `telegram`). Crosses verbatim
    /// as [`IngestItem::platform`]; see [`chat_data_source`] for how it is
    /// additionally mapped onto a [`DataSource`].
    pub platform: String,
    /// Human-readable channel / group name.
    pub channel_label: String,
    /// Ordered messages (chronological; the adapter sorts defensively).
    pub messages: Vec<ChatMessage>,
}

/// One email in a thread.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailMessage {
    /// Sender address; rendered as the `From:` header and used as the
    /// participant key when bucketing a thread.
    pub from: String,
    /// Primary recipient addresses; the `To:` header (omitted when empty).
    #[serde(default)]
    pub to: Vec<String>,
    /// Carbon-copy recipient addresses; the `Cc:` header (omitted when empty).
    #[serde(default)]
    pub cc: Vec<String>,
    /// Per-message subject; the `Subject:` header.
    pub subject: String,
    /// When the message was sent (epoch-ms integer or RFC 3339 string).
    /// When absent, defaults to `Utc::now()` so one missing key does not
    /// reject the whole thread.
    #[serde(
        default = "ingest_timestamp_now",
        serialize_with = "chrono::serde::ts_milliseconds::serialize",
        deserialize_with = "deserialize_flexible_timestamp"
    )]
    pub sent_at: DateTime<Utc>,
    /// Plain-text or markdown body.
    pub body: String,
    /// Message-id header or provider URL; used for citation back to source.
    #[serde(default)]
    pub source_ref: Option<String>,
    /// `List-Unsubscribe` header. Carried through because an unsubscribe flow
    /// reads it back out of stored mail — dropping it makes that flow
    /// impossible, not merely poorer (see [`email_items`]).
    #[serde(default)]
    pub list_unsubscribe: Option<String>,
}

/// A whole email thread.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailThread {
    /// Provider name (e.g. `gmail`, `outlook`). See [`email_data_source`].
    pub provider: String,
    /// Thread subject (usually the subject of the first message).
    pub thread_subject: String,
    /// Ordered messages (chronological; the adapter sorts defensively).
    pub messages: Vec<EmailMessage>,
}

/// Adapter input for a single document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentInput {
    /// Provider name (e.g. `notion`, `drive`, `meeting_notes`). Defaults to
    /// `"unknown"` when absent. See [`document_data_source`].
    #[serde(default = "default_document_provider")]
    pub provider: String,
    /// Document title. Read only to decide whether the payload was wholly
    /// empty — it does not cross to the driver; see [`document_item`].
    pub title: String,
    /// Document body (markdown preferred; plain text also accepted).
    pub body: String,
    /// When the document was last modified at the source.
    ///
    /// Accepts an epoch-milliseconds integer (back-compat), an RFC 3339 /
    /// ISO-8601 string, or absent → `Utc::now()`.
    #[serde(
        default = "ingest_timestamp_now",
        deserialize_with = "deserialize_flexible_timestamp"
    )]
    pub modified_at: DateTime<Utc>,
    /// Optional pointer back to source (URL, file path, Notion page id).
    #[serde(default)]
    pub source_ref: Option<String>,
}

/// Unified ingest request. The `payload` shape is adapter-specific and is
/// validated inside the dispatch based on `source_kind`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestRequest {
    /// Which kind of source the payload represents.
    pub source_kind: SourceKind,
    /// Logical source id (channel/group for chat, thread for email, doc id).
    pub source_id: String,
    /// Account/user this content belongs to.
    #[serde(default)]
    pub owner: String,
    /// Optional labels/tags carried through.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Adapter-specific payload — shape matches the canonicaliser for
    /// `source_kind`:
    /// - `chat`     → [`ChatBatch`]
    /// - `email`    → [`EmailThread`]
    /// - `document` → [`DocumentInput`]
    pub payload: Value,
}

/// Response body of the `memory_tree_ingest` RPC.
///
/// Declared here rather than returned as the engine's own summary type,
/// because this is a wire shape the frontend reads: a body owned by a foreign
/// crate is one an upstream field rename can reshape without anything in this
/// repository failing to compile. Every key and JSON type is what that summary
/// serialised and must stay that way —
/// `the_response_body_serialises_exactly_as_the_engine_summary` is the pin, and
/// it is what the chat and document arms' move onto the driver contract had to
/// keep true. Those two build this body from an `IngestOutcome` now, mail still
/// from the pipeline's summary, and both spell the same six keys.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestResponse {
    /// Logical source id the ingest was scoped to — the one the caller
    /// supplied, echoed back so a batched caller can pair a reply with its
    /// request.
    pub source_id: String,
    /// Units persisted by this call.
    pub chunks_written: usize,
    /// Units produced and not admitted. Dropped units only: a call refused
    /// outright is [`Self::already_ingested`], not a drop of everything.
    pub chunks_dropped: usize,
    /// Ids of the units this call produced. A caller fetches a chunk back by
    /// these, so a write that names none is unusable even when the count is
    /// right.
    pub chunk_ids: Vec<String>,
    /// Follow-up extraction jobs this call scheduled. Read next to
    /// [`Self::chunks_written`] it answers whether the material just handed
    /// over will be picked up at all — rows can land with nothing scheduled to
    /// derive from them, and the write count alone reports that as success.
    pub extract_jobs_enqueued: usize,
    /// True when the call was a no-op because `(source_kind, source_id)` had
    /// been ingested before.
    ///
    /// Distinct from a zero-write result, and the distinction is the point:
    /// only a refusal is a reason to go and clear the source gate. The gate is
    /// keyed on the logical source rather than on the content, so re-sending
    /// *changed* material under a claimed `source_id` also writes nothing.
    pub already_ingested: bool,
}

/// Build the validation error returned when an ingest payload does not match
/// the canonicaliser schema for its `source_kind`.
///
/// Kept as the single construction site so the wording cannot drift away from
/// [`is_invalid_ingest_payload_message`], which the transport layer uses to
/// pick the Sentry severity. Same emit-site/classifier pairing as
/// `dispatch::UNKNOWN_METHOD_PREFIX` / `dispatch::unknown_method_name`.
pub(crate) fn invalid_payload_message(source_kind: SourceKind, err: &serde_json::Error) -> String {
    format!("invalid {} payload: {err}", source_kind.as_str())
}
