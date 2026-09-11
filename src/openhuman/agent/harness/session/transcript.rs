//! Session transcript persistence for KV cache stability.
//!
//! **Source of truth**: `session_raw/{stem}.jsonl` — a *flat* directory.
//!
//! Each JSONL file starts with a single metadata line (identified by an
//! `_meta` key) followed by one JSON object per record. On every
//! write the companion `.md` file is re-rendered for human readability
//! under `sessions/{YYYY_MM_DD}/{stem}.md`; it is **never** read back —
//! all round-trip / resume logic uses the JSONL.
//!
//! ## Append-only log (Phase A — transcript-derived view)
//!
//! The JSONL is an **append-only event log**. [`write_transcript`] still
//! full-rewrites (used by one-shot writers: migrations, sub-agent runners,
//! tests), but the incremental session persistence path
//! (`persist_session_transcript`) uses [`append_transcript_turn`], which
//! never rewrites existing lines. It classifies the incoming logical
//! message set against what was last persisted:
//!
//! - **Pure extension** (previously-persisted messages are a prefix of the
//!   new set): only the new tail message lines are appended.
//! - **Reduction / rewrite** (context reduction dropped or replaced earlier
//!   turns — the previously-persisted set is *not* a prefix): a single
//!   `{"kind":"compaction","replacement":[…]}` record is appended carrying
//!   the full reduced message set. Earlier turns stay on disk untouched.
//!
//! Cumulative `_meta` totals are kept fresh without rewriting the file by
//! appending a fresh `{"_meta":{…}}` line each turn; readers take the **last**
//! `_meta` line as authoritative (line 1 remains a valid fallback for old
//! cores that only read the header).
//!
//! ### Two read paths
//!
//! - **Model context** ([`read_transcript`] / `read_transcript_jsonl`) replays
//!   the log: message lines accumulate, a `compaction` record **replaces** the
//!   accumulator with its `replacement`, and `interrupted:true` partial lines
//!   are **skipped** (they never entered the model's context). The result is
//!   byte-identical to what the old full-rewrite approach produced.
//! - **Display** ([`read_transcript_display`]) returns **every** record in file
//!   order — pre-compaction history, compaction markers, and interrupted
//!   partials — so the UI projection (Phase B) can render the full timeline.
//!
//! ### Compatibility
//!
//! Existing files (zero compaction records, no `version`) read identically.
//! New record kinds and fields are additive: [`MessageLine`] carries a
//! `#[serde(flatten)] _extra` catch-all and `MetaPayload` does not set
//! `deny_unknown_fields`, so an **old** core reading a **new** file skips
//! unknown-kind lines (a compaction record fails the `role`/`content`
//! requirement and is logged+skipped) rather than crashing. The `_meta`
//! carries a `version` field (`TRANSCRIPT_SCHEMA_VERSION`) for future readers.
//!
//! ## Storage layout
//!
//! ```text
//! {workspace}/session_raw/{stem}.jsonl              ← source of truth (flat)
//! {workspace}/sessions/YYYY_MM_DD/{stem}.md         ← human-readable view
//! ```
//!
//! `stem` is `{unix_ts}_{agent_id}` for a root session, or
//! `{parent_chain}__{unix_ts}_{agent_id}` for a sub-agent. Because the
//! stem starts with the unix timestamp at agent-build time, a directory
//! listing of `session_raw/` is naturally sorted by creation time and
//! `find_latest_transcript` becomes O(scan one dir, filter by suffix)
//! — it does not depend on the calendar date, so a session that's been
//! idle for weeks resumes the same way as one from yesterday.
//!
//! ## Backward compatibility
//!
//! Older releases wrote into `session_raw/DDMMYYYY/{stem}.jsonl` (and
//! the legacy `sessions/DDMMYYYY/{stem}.md`). [`find_latest_transcript`]
//! falls back to scanning those date-grouped dirs when the flat
//! directory yields nothing, so users upgrading don't lose resume.
//!
//! ## JSONL schema
//!
//! **Line 1 (meta):**
//! ```json
//! {"_meta":{"agent":"code_executor","dispatcher":"native","created":"...","updated":"...","turn_count":3,"input_tokens":5000,"output_tokens":1200,"cached_input_tokens":3500,"charged_amount_usd":0.0045,"thread_id":"thr_abc123"}}
//! ```
//!
//! **Message lines:**
//! ```json
//! {"role":"system","content":"..."}
//! {"role":"user","content":"..."}
//! {"role":"assistant","content":"...","model":"claude-...","usage":{"input":1234,"output":567,"cached_input":1000,"cost_usd":0.0012},"ts":"2026-04-17T..."}
//! {"role":"tool","content":"..."}
//! ```
//!
//! Only `role` and `content` are required. All other fields are optional.
//! UI-visible rows may also carry a stable `id` and `extra_metadata` so
//! the session transcript can eventually replace the separate thread
//! message log without losing message-level addressing.

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
include!("transcript_part_01.rs");
include!("transcript_part_02.rs");
include!("transcript_part_03.rs");
