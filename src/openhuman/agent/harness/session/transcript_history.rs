//! [`ChatHistory`] over OpenHuman's durable `session_raw` transcript.
//!
//! This is the seam chosen in `docs/specs/2026-07-28-agent-session-transcript-to-tinyagents-design.md`
//! §4 Option A: the harness talks to the crate's
//! [`tinyagents_harness::memory::ChatHistory`] trait, while OpenHuman keeps
//! ownership of the on-disk format. Nothing about `session_raw` moves, so
//! there is no on-disk change and no migration risk — the previous parallel
//! abstraction is what goes away.
//!
//! [`SessionTranscriptHistory`] is a thin handle over the free functions in
//! [`super::transcript`]. It holds no state beyond the transcript's identity,
//! so it is cheap to construct per turn and safe to share.
//!
//! # Why the trait's `thread_id` is not used for path resolution
//!
//! `ChatHistory` is keyed by `thread_id`, but a `session_raw` transcript is
//! keyed by its **stem** (`{unix_ts}_{agent_id}`, or `{parent_chain}__…` for a
//! sub-agent). These are deliberately different: several transcripts can share
//! one `_meta.thread_id` — every sub-agent spawned within a thread does — so
//! resolving a path from `thread_id` would be ambiguous and could interleave
//! two agents' histories into one file.
//!
//! The handle is therefore bound to one transcript at construction, and the
//! `thread_id` argument is accepted for trait conformance only. Callers that
//! genuinely want thread-level lookup use
//! [`super::transcript::find_root_transcript_for_thread`].
//!
//! # Append-only, including `clear`
//!
//! Every mutation routes through
//! [`append_transcript_turn`][super::transcript::append_transcript_turn], which
//! never rewrites existing lines. A reduction in the logical message set
//! becomes a `{"kind":"compaction","replacement":[…]}` record rather than a
//! file rewrite. That includes [`SessionTranscriptHistory::clear`] — see its
//! doc comment for the semantics that were chosen and why.
//!
//! # Why the metadata-bearing write does not cross the crate trait
//!
//! S4 of the design doc is worded as "the turn path takes `Arc<dyn
//! ChatHistory>`", but that wording conflicts with S4's own exit criterion
//! ("`threads/transcript_view` projection output unchanged"). The crate trait
//! (`vendor/tinyagents/src/harness/memory/types.rs`) has exactly four methods,
//! and every one of them carries only a `thread_id: &str` plus `Message` /
//! `Vec<Message>`. Three things the turn path persists therefore have **no
//! channel**:
//!
//! - **`request_id`** — stamped on every line of a turn. It drives
//!   `DisplayItem::TurnBoundary` (`threads/transcript_view/project.rs`,
//!   `maybe_emit_turn_boundary`) and the `(request_id, ts)` root-turn segments
//!   that anchor every `DisplayItem::Subagent`. Lose it and the transcript view
//!   silently stops showing turn structure.
//! - **`turn_usage`** — attributed to the last assistant row. It carries
//!   `model`, `iteration`, `ts`, `reasoning_content` and the native
//!   `tool_calls`. The projection reads **every** `DisplayItem::ToolCall` off
//!   `turn_usage.tool_calls`, so losing it does not degrade the tool rows, it
//!   deletes them — after which each following `role:"tool"` line falls through
//!   to the orphan branch. `Reasoning` items vanish with it, and
//!   `AssistantMessage.{model,iteration}` / `interim` collapse.
//! - **`TranscriptMeta`'s cumulative fields** — `turn_count` and the four
//!   token/cost rollups that `read_thread_usage_summary` (`threads/ops.rs`)
//!   reports. The turn path computes these fresh each turn;
//!   [`SessionTranscriptHistory::meta_for_write`] deliberately re-reads the
//!   file's existing `_meta` instead, which is right for the generic trait path
//!   but would freeze the rollups at the previous turn's values if the turn
//!   path used it.
//!
//! So the turn path goes through [`SessionHistory::append_turn`] — an
//! OpenHuman-side supertrait of `ChatHistory` whose one method forwards the
//! same six arguments `append_transcript_turn` already takes. The indirection
//! is real (`Arc<dyn SessionHistory>`), the on-disk bytes are unchanged by
//! construction, and `ChatHistory` stays in the bound so the handle is still a
//! genuine crate-side history for any future consumer.
//!
//! ## Two alternatives, rejected — recorded so they are not re-litigated
//!
//! 1. **Per-message `ChatHistory::append`.** One `append_transcript_turn` per
//!    message means one `_meta` line and one full-file re-read per message:
//!    an on-disk change *and* O(n²) I/O on a file that grows without bound.
//! 2. **Widening `ChatHistory` upstream.** It does not close the gap either.
//!    The crate's `Usage` has no `cost_usd` / `context_window`;
//!    `TranscriptMeta` is a cumulative *file header*, not turn provenance; and
//!    the per-message tool-failure `extra_metadata` that
//!    `message_to_chat_message` drops is untouchable by any turn-level record.
//!    You would pay a tinyagents release and still need a
//!    `serde_json::Value` escape hatch, for a trait that has no consumer inside
//!    the vendored crate outside `harness/memory/`.
//!
//! # Why the READ half is [`SessionTranscriptRead`], not `ChatHistory::messages`
//!
//! Same shape of argument as the write, and equally settled — measured with a
//! round-trip probe, not assumed. `ChatHistory::messages` returns
//! `Vec<Message>`, and the turn path needs `Vec<ChatMessage>` back, so a
//! trait-mediated read has to pass through
//! [`message_to_chat_message`][crate::openhuman::agent::message_convert::message_to_chat_message].
//! That converter maps `Message::Assistant` to `ChatMessage::assistant(msg.text())`
//! and **drops `a.tool_calls` entirely**. A persisted native tool round is
//! deliberately stored as the `{content, tool_calls}` / `{tool_call_id, content}`
//! envelope so the next turn re-parses it; flattening it orphans every following
//! `role:"tool"` row and produces the provider `400 An assistant message with
//! 'tool_calls' must be followed by tool messages`. It also blinds the
//! TAURI-RUST-7 trailing strip in
//! [`bound_cached_transcript_messages`][super::types::Agent::bound_cached_transcript_messages],
//! which sniffs that envelope out of `ChatMessage.content`. Two lesser losses
//! ride along and are inert on this path: the `openhuman_turn_usage`
//! `extra_metadata` (re-attached by `read_transcript`, never re-serialised from
//! the cached prefix) and `AssistantMessage.id` (no reader anywhere).
//!
//! So the read goes through [`SessionTranscriptRead::read_session`], which
//! returns the very [`SessionTranscript`] the free function returns, produced by
//! the same [`read_transcript`] call. Losslessness is **structural**: nothing
//! crosses `Message`, so `tool_calls`, `tool_call_id`, `failure`,
//! `reasoning_content` and the `_meta` header all survive by construction, and
//! compaction replay + `interrupted: true` partial skipping stay exactly where
//! the format owner performs them.
//!
//! # Why discovery is a separate object ([`SessionHistoryLocator`])
//!
//! A handle is bound to one *file*. The turn path's two reads are *lookups*:
//! `(workspace, session_raw_subdir, agent name)` → newest match, and
//! `_meta.thread_id` → newest **root** transcript. `ChatHistory` has no
//! discovery concept at all (it is `thread_id`-keyed and returns messages, never
//! a location), so leaving discovery as free functions would keep the read half
//! hitting the filesystem no matter what handle was injected — i.e. the
//! `Arc<dyn …>` would stay decorative. The locator is therefore the single
//! injected object covering *both* reads and the session's own write handle.
//!
//! # Deliberately NOT done here — recorded with reasons
//!
//! - **The `impl ChatHistory` block below still has no production caller.**
//!   Reads go through `read_session`, writes through `append_turn`. It is kept,
//!   not deleted, because it is the crate-side seam Option A exists to
//!   establish, and because it supplies the `Send + Sync + 'static` bounds the
//!   shared `Arc<dyn SessionHistory>` needs. The trigger that would delete it is
//!   an explicit decision to drop `ChatHistory` from the [`SessionHistory`]
//!   bound; that frees this file's `read`/`persisted`/`meta_for_write`/
//!   `write_logical_set`/`impl ChatHistory` (~150 lines) plus most of the test
//!   module (~570 lines together). Decide it, don't rediscover it.
//! - **The spec's "Removes: ~400 LOC of parallel abstraction" is not delivered
//!   and cannot be.** See the design doc's "Where '~400 LOC' came from"
//!   subsection: the figure is §2.1's residual after Option B, i.e. exactly
//!   `migration.rs` (373 LOC), which §5 S1 and the deletion ledger both keep
//!   host-owned. Option A's measured ledger is ≈ −15 / +90 LOC here.
//! - **The #4249 JSONL↔store mirror is the one genuine parallel session
//!   persistence** (`session_import/live.rs`, `maybe_shadow_read_session_store`
//!   / `maybe_dual_write_session_store`, the `StoreRegistry` registration, two
//!   `AgentConfig` flags, one config migration — ~565 prod LOC). It is not
//!   touched here: it is gated on #4249's own Phase-2 parity soak and its
//!   terminus (reads served from the store) points the opposite way from this
//!   branch's non-negotiable zero-on-disk-change constraint.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::memory::ChatHistory;
use tinyagents_harness::{Result as TaResult, TinyAgentsError};
use tinyinference::message::Message;

use crate::openhuman::agent::message_convert::{history_to_messages, message_to_chat_message};
use crate::openhuman::agent::messages::ChatMessage;

use super::transcript::{
    append_transcript_turn, find_latest_transcript_in_subdir, find_root_transcript_for_thread,
    read_transcript, resolve_keyed_transcript_path, resolve_keyed_transcript_path_in_dir,
    SessionTranscript, TranscriptMeta, TurnUsage,
};

/// One turn's worth of transcript write, borrowed.
///
/// The fields mirror [`append_transcript_turn`]'s argument list one-for-one and
/// in order, so [`SessionHistory::append_turn`]'s forwarding is visually
/// checkable against the format's own signature. Nothing is transformed on the
/// way through; that is the entire correctness claim of this seam and
/// `append_turn_is_byte_identical_to_the_free_function` in the tests pins it.
///
/// `prev` is a field rather than handle state on purpose: the turn path tracks
/// the previously-persisted logical set in memory on `Agent`
/// (`persisted_transcript_messages`) precisely so it never has to re-read a
/// growing file, and a disk re-read is not a faithful substitute — see
/// [`SessionTranscriptHistory::write_logical_set`].
pub struct TranscriptTurn<'a> {
    /// Logical message set already persisted, for the extension-vs-compaction diff.
    pub prev: &'a [ChatMessage],
    /// Logical message set after this turn.
    pub next: &'a [ChatMessage],
    /// `_meta` header to append after this turn's lines.
    pub meta: &'a TranscriptMeta,
    /// Usage + provenance attributed to the turn's last assistant row.
    pub turn_usage: Option<&'a TurnUsage>,
    /// Web-chat request id, stamped on every line of the turn.
    pub request_id: Option<&'a str>,
}

/// The seam the live turn path holds as `Arc<dyn SessionHistory>`.
///
/// `ChatHistory` is a supertrait rather than a sibling for two reasons: it
/// supplies the `Send + Sync + 'static` bounds the shared handle needs, and it
/// keeps the crate-side surface (S2/S3) live rather than orphaned. See this
/// module's header for why the turn write cannot simply *be* a `ChatHistory`
/// call.
///
/// `append_turn` is deliberately **sync**: `persist_session_transcript` is a
/// sync `&mut self` method and the whole write chain under it is sync, so an
/// async method here would ripple `.await` through the turn loop for no gain.
pub(crate) trait SessionHistory: ChatHistory + SessionTranscriptRead {
    /// Appends one turn, forwarding every argument to the format owner.
    fn append_turn(&self, turn: TranscriptTurn<'_>) -> anyhow::Result<()>;
}

/// The read half of a bound transcript — the seam the turn path's two resume
/// reads hold.
///
/// Split out of [`SessionHistory`] rather than added as one more method on it,
/// for a reason that is not stylistic: a *discovered* transcript can still be a
/// legacy `.md` file (see [`SessionTranscriptHistory::opened_at`]), and
/// `append_transcript_turn` writes JSONL. Handing discovery results out as
/// `Arc<dyn SessionTranscriptRead>` makes it impossible to `append_turn` into
/// one by construction, instead of by convention.
///
/// Sync for the same reason [`SessionHistory::append_turn`] is: both callers are
/// sync `&mut self` methods on `Agent`.
pub(crate) trait SessionTranscriptRead: Send + Sync {
    /// The transcript file this handle is bound to.
    ///
    /// The turn path still needs the concrete path after the read:
    /// `maybe_shadow_read_session_store` takes `&Path`, and the dual-write
    /// mirror derives its record key from `file_stem()`.
    fn path(&self) -> &Path;

    /// The model-context replay of this transcript, `_meta` included, or
    /// `Ok(None)` when the file does not exist.
    ///
    /// Exactly [`read_transcript`], so compaction records have already replaced
    /// the accumulator and `interrupted: true` partials are already skipped —
    /// §3.1's "single most important constraint". Returning the whole
    /// [`SessionTranscript`] rather than messages alone is what lets the shadow
    /// read keep working through this seam.
    fn read_session(&self) -> anyhow::Result<Option<SessionTranscript>>;
}

/// Resolves transcripts by the two keys the turn path actually has, and binds
/// this session's own write handle.
///
/// One injected object covers the whole turn path: both resume reads and the
/// first-write bind. `Agent` holds it as `Option<Arc<dyn SessionHistoryLocator>>`
/// and falls back to [`FileTranscriptLocator`] built from the *current*
/// `workspace_dir`/`session_raw_subdir` — lazily, never frozen at build time,
/// because tests reassign `agent.workspace_dir` after `build()` and a
/// build-time locator would silently keep pointing at the old directory.
pub(crate) trait SessionHistoryLocator: Send + Sync {
    /// Newest transcript for `agent_name` in this session's raw subtree,
    /// including the legacy `session_raw/DDMMYYYY/` + `.md` fallback.
    fn latest_for_agent(&self, agent_name: &str) -> Option<Arc<dyn SessionTranscriptRead>>;

    /// Newest **root** transcript whose `_meta.thread_id` matches.
    ///
    /// Root-only on purpose: several transcripts share one thread id (every
    /// sub-agent spawned within it does), so a stem-keyed lookup would be
    /// ambiguous.
    fn root_for_thread(&self, thread_id: &str) -> Option<Arc<dyn SessionTranscriptRead>>;

    /// Binds (creating on first write) this session's own write handle for
    /// `stem`, with `seed` used only when no file exists yet.
    fn open_stem(
        &self,
        stem: &str,
        seed: TranscriptMeta,
    ) -> anyhow::Result<Arc<dyn SessionHistory>>;
}

/// The default [`SessionHistoryLocator`]: real files under
/// `{workspace_dir}/{session_raw_subdir}`.
///
/// Thin by design — each method wraps exactly one `transcript::` free function
/// and changes nothing about it, so swapping the turn path onto the locator is
/// behaviour-preserving.
pub(crate) struct FileTranscriptLocator {
    workspace_dir: PathBuf,
    session_raw_subdir: String,
}

impl FileTranscriptLocator {
    pub(crate) fn new(
        workspace_dir: impl Into<PathBuf>,
        session_raw_subdir: impl Into<String>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            session_raw_subdir: session_raw_subdir.into(),
        }
    }

    /// `{workspace_dir}/{session_raw_subdir}` — the profile-scoped raw dir.
    fn raw_dir(&self) -> PathBuf {
        self.workspace_dir.join(&self.session_raw_subdir)
    }
}

impl SessionHistoryLocator for FileTranscriptLocator {
    fn latest_for_agent(&self, agent_name: &str) -> Option<Arc<dyn SessionTranscriptRead>> {
        let path = find_latest_transcript_in_subdir(
            &self.workspace_dir,
            &self.session_raw_subdir,
            agent_name,
        )?;
        log::debug!(
            "[transcript-history] locator latest_for_agent agent={agent_name} path={}",
            path.display()
        );
        Some(Arc::new(SessionTranscriptHistory::opened_at(
            path,
            seed_meta_for_discovered(agent_name),
        )))
    }

    fn root_for_thread(&self, thread_id: &str) -> Option<Arc<dyn SessionTranscriptRead>> {
        // Cross-dir, newest-wins — NOT scoped to this locator's own
        // `session_raw-<id>/` (#5351). A thread's conversation belongs to the
        // THREAD, not the active profile, so this scans the shared
        // `session_raw/` and every profile-scoped sibling and takes the newest
        // match. Switching profile mid-thread (the Quick/Reasoning toggle) then
        // continues the same conversation even when earlier turns were written
        // under another profile's subtree.
        //
        // Deliberately not own-dir-first: that lets an OLDER transcript in this
        // agent's own dir shadow a NEWER one a sibling holds for the same
        // thread, dropping recent turns and diverging from the transcript view
        // and turn mirror, which both use this same resolver. The own dir is
        // already in the scan, so newest-wins is a superset.
        let path = find_root_transcript_for_thread(&self.workspace_dir, thread_id)?;
        log::debug!(
            "[transcript-history] locator root_for_thread thread={thread_id} path={}",
            path.display()
        );
        Some(Arc::new(SessionTranscriptHistory::opened_at(
            path,
            seed_meta_for_discovered(thread_id),
        )))
    }

    fn open_stem(
        &self,
        stem: &str,
        seed: TranscriptMeta,
    ) -> anyhow::Result<Arc<dyn SessionHistory>> {
        // `new_in_dir` — never `new` — because `new` hardcodes
        // `{workspace}/session_raw/`, and a dedicated-memory profile's sessions
        // live in `session_raw-<id>/`.
        Ok(Arc::new(SessionTranscriptHistory::new_in_dir(
            self.raw_dir(),
            stem,
            seed,
        )?))
    }
}

/// A placeholder `_meta` for a handle bound to an already-existing transcript.
///
/// `seed_meta` is consulted only when the file is **absent**, and a discovered
/// path exists by definition, so this value is never written. It exists because
/// [`SessionTranscriptHistory`] is one type serving both roles; giving read-only
/// handles a `None` meta would mean an `Option` field every write path then has
/// to unwrap for no benefit.
fn seed_meta_for_discovered(agent_name: &str) -> TranscriptMeta {
    TranscriptMeta {
        agent_name: agent_name.to_string(),
        agent_id: None,
        agent_type: None,
        dispatcher: String::new(),
        provider: None,
        model: None,
        created: String::new(),
        updated: String::new(),
        turn_count: 0,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: None,
        task_id: None,
    }
}

/// A [`ChatHistory`] backed by one `session_raw/{stem}.jsonl` transcript.
///
/// Construct with [`SessionTranscriptHistory::new`] (workspace-rooted, i.e.
/// `{workspace}/session_raw/`) or [`SessionTranscriptHistory::new_in_dir`] (an
/// explicit raw dir — **required** for a dedicated-memory profile, whose
/// sessions live in `session_raw-<id>/`). The `seed_meta` is used only when the
/// transcript file does not exist yet; for an existing file the authoritative
/// cumulative `_meta` is read back from disk so turn counts and token rollups
/// keep accumulating rather than resetting.
pub struct SessionTranscriptHistory {
    /// Fully-resolved transcript file, fixed at construction.
    ///
    /// Resolved eagerly rather than derived per call from a `(workspace, stem)`
    /// pair: the old shape hardcoded `{workspace}/session_raw/`, which is the
    /// **wrong directory** for a profile-scoped session and would have silently
    /// cross-written into the shared profile's transcripts the moment this
    /// handle was wired into the turn path.
    path: PathBuf,
    /// `_meta` used for the very first write, before a file exists.
    seed_meta: TranscriptMeta,
}

impl SessionTranscriptHistory {
    /// Binds a history handle to `{workspace_dir}/session_raw/{stem}.jsonl`.
    ///
    /// Use [`Self::new_in_dir`] when the session is profile-scoped; this
    /// convenience constructor always resolves under the shared `session_raw/`.
    pub fn new(
        workspace_dir: impl AsRef<Path>,
        stem: &str,
        seed_meta: TranscriptMeta,
    ) -> anyhow::Result<Self> {
        let path = resolve_keyed_transcript_path(workspace_dir.as_ref(), stem)?;
        log::debug!(
            "[transcript-history] bound stem={stem} path={}",
            path.display()
        );
        Ok(Self { path, seed_meta })
    }

    /// Binds a history handle to `{session_raw_dir}/{stem}.jsonl`.
    ///
    /// `session_raw_dir` is `{workspace}/{session_raw_subdir}` — `session_raw`
    /// for the shared profile, `session_raw-<id>` for a dedicated-memory one.
    /// The turn path must use this constructor; see [`Self::path`]'s note.
    pub fn new_in_dir(
        session_raw_dir: impl AsRef<Path>,
        stem: &str,
        seed_meta: TranscriptMeta,
    ) -> anyhow::Result<Self> {
        let path = resolve_keyed_transcript_path_in_dir(session_raw_dir.as_ref(), stem)?;
        log::debug!(
            "[transcript-history] bound stem={stem} path={}",
            path.display()
        );
        Ok(Self { path, seed_meta })
    }

    /// Binds a handle to an **already-discovered** transcript file, verbatim.
    ///
    /// Deliberately does **not** go through `resolve_keyed_transcript_path*`,
    /// which the two stem constructors above use. That helper `create_dir_all`s
    /// its parent and forces a `.jsonl` extension — both wrong for a discovered
    /// path: `find_latest_transcript_in_subdir` can still return a legacy `.md`
    /// file (`read_transcript` routes by extension), and re-resolving would
    /// mangle it into a sibling `.jsonl` that does not exist while creating
    /// stray directories on a pure read.
    ///
    /// Hand the result out as `Arc<dyn SessionTranscriptRead>`, not
    /// `Arc<dyn SessionHistory>` — see [`SessionTranscriptRead`]'s doc.
    pub fn opened_at(path: PathBuf, seed_meta: TranscriptMeta) -> Self {
        log::debug!(
            "[transcript-history] opened discovered path={}",
            path.display()
        );
        Self { path, seed_meta }
    }

    /// This handle's transcript file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the current transcript, or `None` when no file exists yet.
    ///
    /// A missing transcript is the normal first-turn state, not an error.
    fn read(&self) -> TaResult<Option<SessionTranscript>> {
        if !self.path.exists() {
            return Ok(None);
        }
        read_transcript(&self.path).map(Some).map_err(memory_err)
    }

    /// The logical (model-context) message set currently on disk.
    ///
    /// Routes through [`read_transcript`], so compaction records have already
    /// replaced the accumulator and `interrupted: true` partials are skipped.
    fn persisted(&self) -> TaResult<Vec<ChatMessage>> {
        Ok(self.read()?.map(|t| t.messages).unwrap_or_default())
    }

    /// The `_meta` to write: the file's own cumulative meta when it exists,
    /// otherwise this handle's seed.
    ///
    /// Correct for the generic `ChatHistory` path, which has no channel for a
    /// caller-computed meta. The **turn path must never route through here** —
    /// it computes `turn_count` and the four token/cost rollups fresh each turn,
    /// and re-reading the file's `_meta` would freeze them at the previous
    /// turn's values, silently breaking `read_thread_usage_summary`.
    fn meta_for_write(&self) -> TaResult<TranscriptMeta> {
        Ok(self
            .read()?
            .map(|t| t.meta)
            .unwrap_or_else(|| self.seed_meta.clone()))
    }

    /// Writes `next` as the new logical set, diffing against what is persisted.
    ///
    /// Routes through [`SessionHistory::append_turn`] so every write in this
    /// module — trait-driven and turn-path alike — funnels through one call to
    /// [`append_transcript_turn`], and the extension-vs-compaction decision
    /// stays with the format owner rather than drifting here.
    ///
    /// The `self.persisted()` disk re-read is what the generic trait path has
    /// to do, and is deliberately **not** what the turn path does.
    /// [`read_transcript`] reconstructs `ChatMessage`s from line records: the
    /// `failure` / `failure_detail` fields have been lifted out of
    /// `extra_metadata` and turn-usage fields hoisted to top-level line fields.
    /// Feeding that back in as `prev` would make `common_prefix_len` mismatch
    /// at the first such message, so the writer would emit a full compaction
    /// record — re-appending the entire message set — on every single turn.
    fn write_logical_set(&self, next: &[ChatMessage]) -> TaResult<()> {
        let prev = self.persisted()?;
        let meta = self.meta_for_write()?;
        self.append_turn(TranscriptTurn {
            prev: &prev,
            next,
            meta: &meta,
            turn_usage: None,
            request_id: None,
        })
        .map_err(memory_err)
    }
}

impl SessionTranscriptRead for SessionTranscriptHistory {
    fn path(&self) -> &Path {
        &self.path
    }

    /// Same call the free-function readers make, on the same path, with the
    /// same return type — so there is nothing left for the round trip to lose.
    fn read_session(&self) -> anyhow::Result<Option<SessionTranscript>> {
        if !self.path.exists() {
            log::debug!(
                "[transcript-history] read_session absent path={}",
                self.path.display()
            );
            return Ok(None);
        }
        let session = read_transcript(&self.path)?;
        log::debug!(
            "[transcript-history] read_session messages={} path={}",
            session.messages.len(),
            self.path.display()
        );
        Ok(Some(session))
    }
}

impl SessionHistory for SessionTranscriptHistory {
    /// Pure forwarder: every argument reaches [`append_transcript_turn`]
    /// untouched, so the bytes this writes are identical to what the free
    /// function would have written at the call site.
    fn append_turn(&self, turn: TranscriptTurn<'_>) -> anyhow::Result<()> {
        log::debug!(
            "[transcript-history] append_turn prev={} next={} usage={} request_id={:?} path={}",
            turn.prev.len(),
            turn.next.len(),
            turn.turn_usage.is_some(),
            turn.request_id,
            self.path.display()
        );
        append_transcript_turn(
            &self.path,
            turn.prev,
            turn.next,
            turn.meta,
            turn.turn_usage,
            turn.request_id,
        )
    }
}

#[async_trait]
impl ChatHistory for SessionTranscriptHistory {
    /// Returns the **model-context** replay of this transcript.
    ///
    /// This is deliberately the same path the resume flow uses, not the raw
    /// line set: compaction records replace the accumulator and interrupted
    /// partials are dropped, so a resumed context never carries a truncated
    /// answer. Use
    /// [`read_transcript_display`][super::transcript::read_transcript_display]
    /// when rendering history for a human instead.
    ///
    /// An absent transcript yields an empty `Vec`, per the trait contract.
    async fn messages(&self, _thread_id: &str) -> TaResult<Vec<Message>> {
        Ok(history_to_messages(&self.persisted()?))
    }

    /// Appends one message to the end of the transcript.
    ///
    /// Extending the persisted set writes only the new tail line.
    async fn append(&self, _thread_id: &str, message: Message) -> TaResult<()> {
        let mut next = self.persisted()?;
        next.push(message_to_chat_message(&message));
        self.write_logical_set(&next)
    }

    /// Replaces the logical message set with `messages`.
    ///
    /// This maps onto the compaction-record path, **not** a file rewrite: when
    /// `messages` is no longer an extension of what is on disk,
    /// [`append_transcript_turn`] appends a single
    /// `{"kind":"compaction","replacement":[…]}` record carrying the full
    /// reduced set and leaves every earlier line in place. The trait's default
    /// implementation (clear-then-append) would destroy that history, which is
    /// why this override exists.
    async fn replace(&self, _thread_id: &str, messages: Vec<Message>) -> TaResult<()> {
        let next: Vec<ChatMessage> = messages.iter().map(message_to_chat_message).collect();
        self.write_logical_set(&next)
    }

    /// Empties the **model context** while preserving the transcript on disk.
    ///
    /// Semantics chosen (S3 requires this be explicit): `clear` appends a
    /// compaction record with an empty `replacement`. Afterwards
    /// [`messages`][Self::messages] returns empty, but every prior line — and
    /// so the display read, usage rollups, and audit trail — survives.
    ///
    /// The two rejected alternatives, recorded so this is not re-litigated:
    /// truncating the file breaks the append-only invariant that the whole
    /// format rests on, and starting a fresh stem would silently orphan the
    /// session's history from its thread. A no-op on an absent transcript, per
    /// the trait contract.
    async fn clear(&self, _thread_id: &str) -> TaResult<()> {
        if !self.path.exists() {
            return Ok(());
        }
        self.write_logical_set(&[])
    }
}

/// Maps a transcript I/O failure into the crate's error type.
///
/// `ChatHistory` is a `harness::memory` surface, so its failures classify as
/// [`TinyAgentsError::Memory`]. The `anyhow` context chain is flattened into
/// the message via `{:#}` so the underlying cause is not lost.
fn memory_err(err: anyhow::Error) -> TinyAgentsError {
    TinyAgentsError::Memory(format!("session transcript: {err:#}"))
}

#[cfg(test)]
#[path = "transcript_history_tests.rs"]
mod tests;
