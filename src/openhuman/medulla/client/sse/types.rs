//! Data types for the `sse` module.
#[allow(unused_imports)]
use super::*;
/// A completed SSE frame.
#[derive(Debug, Clone, PartialEq)]
pub struct SseFrame {
    /// Cursor value from an `id:` line, when present.
    pub id: Option<u64>,
    /// Concatenated `data:` payload (lines joined with `\n`).
    pub data: String,
}
/// Incremental SSE line parser. Feed byte chunks; collect completed frames.
#[derive(Debug, Default)]
pub struct SseParser {
    /// Bytes of an incomplete trailing line.
    pub(super) line_buf: String,
    /// Accumulated `data:` payload for the in-progress frame.
    pub(super) data: String,
    /// Whether any `data:` line has been seen for the in-progress frame.
    pub(super) got_data: bool,
    /// `id:` value seen for the in-progress frame.
    pub(super) id: Option<u64>,
}
/// Seq-based de-duplication for reconnect replay.
///
/// Frames carrying a persisted `seq` are only accepted when they advance past
/// the cursor; frames without a seq (deltas) always pass.
#[derive(Debug, Default)]
pub struct SeqDedup {
    pub(super) cursor: Option<u64>,
}
/// Internal driver state for the reconnecting stream.
pub(super) struct StreamState {
    pub(super) http: reqwest::Client,
    pub(super) url: String,
    pub(super) parser: SseParser,
    pub(super) dedup: SeqDedup,
    pub(super) pending: VecDeque<Result<EventEnvelope>>,
    pub(super) body: Option<futures::stream::BoxStream<'static, reqwest::Result<Vec<u8>>>>,
    pub(super) first_connect: bool,
}
