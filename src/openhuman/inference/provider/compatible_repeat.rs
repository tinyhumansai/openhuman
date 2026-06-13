/// `frequency_penalty` applied to streaming chat-completions requests.
///
/// Autoregressive models have a self-reinforcing bias toward repeating spans
/// already in their context; with no penalty a momentary repeat can spiral into
/// the same line emitted until the output-token cap (degenerate decoding). A
/// small positive penalty damps that loop without harming coherence. Carried on
/// the streaming path (where those loops occur — long autonomous turns) and
/// retried without it if a strict provider rejects it; the buffered
/// non-streaming fallback omits it for maximum compatibility. Skipped in
/// serialisation when `None` so providers that don't accept the field are
/// unaffected.
pub(super) const CHAT_FREQUENCY_PENALTY: f64 = 0.3;

/// Consecutive identical substantial lines that trip the in-generation repeat
/// cutoff. Autoregressive models can latch onto a line and emit it verbatim
/// until the token cap (observed: 234× the same sentence in one response).
/// `frequency_penalty` / stronger model tiers only lower the odds — they don't
/// prevent it — so this is the deterministic, model-agnostic stop. Set well
/// above any legitimate repetition.
pub(crate) const STREAM_REPEAT_THRESHOLD: u32 = 6;
/// Minimum trimmed length for a line to count toward [`STREAM_REPEAT_THRESHOLD`].
/// Keeps short, legitimately-repeated lines (`}`, blank-ish code) from tripping
/// it; degenerate spirals are long sentences well over this.
pub(super) const MIN_REPEAT_LINE_CHARS: usize = 16;

/// Detects in-generation repetition degeneration on the streaming path so the
/// reader can abort the stream and truncate the blob. Trips after
/// [`STREAM_REPEAT_THRESHOLD`] consecutive identical substantial lines; blank
/// separator lines are ignored, so `"sentence\n\nsentence\n\n…"` still trips.
#[derive(Default)]
pub(crate) struct StreamRepeatDetector {
    current_line: String,
    last_line: Option<String>,
    consecutive: u32,
}

impl StreamRepeatDetector {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Feed one streamed text delta. Returns `true` once the same substantial
    /// line has repeated [`STREAM_REPEAT_THRESHOLD`] times back-to-back.
    pub(super) fn observe(&mut self, delta: &str) -> bool {
        for ch in delta.chars() {
            if ch == '\n' {
                if self.finalize_line() {
                    return true;
                }
            } else {
                self.current_line.push(ch);
            }
        }
        false
    }

    fn finalize_line(&mut self) -> bool {
        let line = self.current_line.trim().to_string();
        self.current_line.clear();
        if line.is_empty() {
            return false;
        }
        if line.chars().count() < MIN_REPEAT_LINE_CHARS {
            self.last_line = Some(line);
            self.consecutive = 1;
            return false;
        }
        if self.last_line.as_deref() == Some(line.as_str()) {
            self.consecutive += 1;
        } else {
            self.last_line = Some(line);
            self.consecutive = 1;
        }
        self.consecutive >= STREAM_REPEAT_THRESHOLD
    }
}
