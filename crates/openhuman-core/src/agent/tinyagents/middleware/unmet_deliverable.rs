//! A named output file that was never written, caught before the turn ends.
//!
//! # Why this exists
//!
//! The turn already carries four rungs that *tell* the model to produce its
//! deliverable: the budget notice part-way through, the penultimate call's
//! narrowed writer belt, the concluding call's instruction, and the
//! requirements check. Every one of them is advice, and the orchestrator
//! prompt's rule ("when the request names an output file, that file is the
//! deliverable") is advice too. None of them observes whether anything was
//! written, so a turn that ignores all five ends with a careful account of
//! work nobody can use.
//!
//! That is not hypothetical. One run reconstructed a transcript, catalogued
//! eleven variants and resolved the domain it was asked for, listed the output
//! file in its own answer as "still outstanding", and ended the turn. Every
//! check that read the file failed on the file's absence rather than on
//! anything it contained; an earlier run of the same request, which wrote a
//! partly-filled file, was scored on its contents.
//!
//! # How a path is identified without parsing the request
//!
//! The obvious objection to reading paths out of a request is telling an
//! output from an input: a request routinely names both. Grammar ("write to"
//! versus "read from") would be the fragile way to decide.
//!
//! The existence check decides it instead, for free. A path the request names
//! as an input is already on disk, so it is never reported. A path that the
//! request names and that nothing has created is, by construction, either a
//! deliverable that was skipped or a path the request mentioned in passing —
//! and the cost of the second case is one advisory sentence.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::{Result, TinyAgentsError};
use tinyagents_harness::middleware::library::TurnClock;
use tinyagents_harness::middleware::Middleware;
use tinyagents_harness::middleware::ToolInvocationIdentity;
use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::tinyinference_llm::message::Message;
use tinyagents_harness::tinyinference_llm::model::{ModelRequest, ModelResponse};
use tinytools::{ToolContent, ToolResult as TaToolResult};

use crate::agent::session_host::turn_checkpoint::wrap_harness_instruction;

/// Leave the model room to act on the notice. Below this the turn cannot both
/// write a file and answer, so the notice would only cost a call.
const MIN_REMAINING_MODEL_CALLS: usize = 3;

/// How many candidates one request may contribute. A request naming more
/// absolute paths than this is describing a tree, not a deliverable, and
/// statting an unbounded list on the concluding call is not worth it.
const MAX_CANDIDATES: usize = 8;

/// Longest plausible path. Anything longer is prose that happens to contain
/// slashes.
const MAX_PATH_CHARS: usize = 200;

/// Characters that continue a path token. Everything else — whitespace,
/// quotes, backticks, brackets, commas — ends it.
fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+' | '@' | '%')
}

/// Whether `segment` ends in a file extension: a dot followed by 1-8
/// alphanumerics, and not the whole segment (so a dotfile is not an
/// extension).
fn has_extension(segment: &str) -> bool {
    match segment.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=8).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

/// Absolute paths with a file extension that `text` names, in order, deduped
/// and capped at [`MAX_CANDIDATES`].
///
/// Absolute only: a relative token in prose ("see config/settings.yml") is far
/// more often a reference than a deliverable, and a relative path cannot be
/// resolved without assuming the turn's working directory. An extension is
/// required for the same reason — it is what separates a file the request asks
/// for from a directory or a sentence fragment. Both limits mean this reports
/// nothing rather than guessing when a request is written loosely.
pub(crate) fn candidate_paths(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != '/' {
            index += 1;
            continue;
        }
        // A path starts at a `/` that does not continue a token (so `a/b` in
        // prose is not read as rooted at `/b`), and not at the `//` of a URL
        // (`https://host/report.pdf` is not a local file).
        if index > 0 && (is_path_char(bytes[index - 1]) || bytes[index - 1] == ':') {
            index += 1;
            continue;
        }
        if bytes.get(index + 1) == Some(&'/') {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_path_char(bytes[index]) {
            index += 1;
        }
        let token: String = bytes[start..index].iter().collect();
        let token = token.trim_end_matches(['.', '-', '_', '+']);
        if token.len() < 3 || token.len() > MAX_PATH_CHARS {
            continue;
        }
        // `..` is never statted: a request that needs traversal to name its
        // own output is not a case this should act on.
        if token.contains("..") {
            continue;
        }
        let mut segments = token.split('/').filter(|s| !s.is_empty());
        let Some(first) = segments.next() else {
            continue;
        };
        let Some(last) = token.rsplit('/').next() else {
            continue;
        };
        // At least two segments: `/report.json` at the filesystem root is far
        // more likely a stray token than a deliverable.
        if first == last || !has_extension(last) {
            continue;
        }
        if seen.insert(token.to_string()) {
            out.push(token.to_string());
            if out.len() >= MAX_CANDIDATES {
                break;
            }
        }
    }
    out
}

/// The notice, naming every candidate that does not exist.
pub(crate) fn notice(missing: &[String]) -> String {
    let list = missing
        .iter()
        .map(|path| format!("`{path}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let (subject, verb) = if missing.len() == 1 {
        ("a file", "does not exist")
    } else {
        ("files", "do not exist")
    };
    wrap_harness_instruction(&format!(
        "The request names {subject} that {verb} here: {list}. If that is the file this turn \
         was asked to produce, nothing has been written there and none of this turn's work \
         is available to whoever asked for it: write it now with whatever you have \
         established, even where fields are incomplete or provisional, and then answer. A \
         file holding the parts you are sure of is worth more than a description of what it \
         would have contained; mark anything provisional inside it, or say in your reply what \
         is still missing. If the path refers to a file that lives elsewhere (another \
         machine, a container, a service), say so and answer as before."
    ))
}

/// Per-run state: the candidates read off the request, and whether the notice
/// has already been given.
#[derive(Default)]
struct RunState {
    candidates: Option<Vec<String>>,
    fired: bool,
    /// The half-time note (a requested path still absent at 50% of the
    /// clock) has been appended to a tool result.
    half_noted: bool,
    /// The late note (80% of the clock: stop exploring, measure every stated
    /// limit) has been appended.
    late_noted: bool,
}

/// Share of the turn's wall clock after which a requested path that still
/// does not exist is pointed out on the next tool result.
const HALF_TIME_BAND: u32 = 5;
/// Share of the turn's wall clock after which exploring stops being worth
/// it and the note says so.
const LATE_BAND: u32 = 8;

/// The note for a requested path still absent at half-time. The deliverable
/// written now and improved in place beats one written in the last minute:
/// one run spent eight of its nine minutes probing an input format, wrote
/// the program at minute fourteen of fifteen, and had no time left to make
/// it meet the size limit the request stated.
fn half_time_note(missing: &[String], clock: &TurnClock) -> String {
    let list = missing
        .iter()
        .map(|path| format!("`{path}`"))
        .collect::<Vec<_>>()
        .join(", ");
    wrap_harness_instruction(&format!(
        "Half the turn's budget is gone ({} left) and nothing exists here yet at {list}. If \
         that is the file this turn was asked to produce, write it now from what you have \
         established, even if provisional, then improve it in place; measure every limit the \
         request states against it as you go. Exploring further before it exists risks \
         ending with nothing to judge. If the path refers to a file that lives elsewhere \
         (another machine, a container, a service), say so in your answer.",
        clock_text(clock.remaining())
    ))
}

/// The note at 80% of the clock, whether or not the deliverable exists.
fn late_note(clock: &TurnClock) -> String {
    wrap_harness_instruction(&format!(
        "{} of the turn's budget left. Stop exploring: make what exists meet every limit, \
         format and interface the request names, measure each one through the exact interface \
         it will be judged by, and finish. A result that meets the stated limits beats a better \
         one that is not written.",
        clock_text(clock.remaining())
    ))
}

/// `1m 30s`-style rendering of what is left on the clock.
fn clock_text(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// Appends `note` after the result's own content, in the plain blocks and in
/// the markdown rendering, the way the harness's own clock note is appended.
fn append_note(result: &mut TaToolResult, note: &str) {
    result.content.push(ToolContent::Text {
        text: format!("\n{note}"),
    });
    if let Some(markdown) = result.markdown_formatted.as_mut() {
        markdown.push('\n');
        markdown.push_str(note);
    }
}

/// Holds a turn's first final answer once, when the request named an output
/// file that nothing has created. See the module docs.
pub(crate) struct UnmetDeliverableMiddleware {
    runs: Mutex<HashMap<u64, RunState>>,
    /// The turn\'s wall-clock budget, handed over at construction like the
    /// other time-note middlewares: the run context does not carry it.
    budget: Option<std::time::Duration>,
}

impl UnmetDeliverableMiddleware {
    pub(crate) fn new(budget: Option<std::time::Duration>) -> Self {
        Self {
            runs: Mutex::default(),
            budget,
        }
    }

    /// Which of `candidates` are not regular files (a directory written at a
    /// `.csv` path is as unmet as nothing at all). Existence only: nothing is
    /// opened or read, and the paths echoed back are the ones the request
    /// already carried.
    fn missing(candidates: &[String]) -> Vec<String> {
        candidates
            .iter()
            .filter(|path| !std::path::Path::new(path.as_str()).is_file())
            .cloned()
            .collect()
    }

    /// Why this response must not be held, or `None` when it may be.
    fn skip_reason<C>(ctx: &RunContext<C>, response: &ModelResponse) -> Option<&'static str> {
        if !response.tool_calls().is_empty() {
            return Some("not_final");
        }
        if response.text().trim().is_empty() {
            return Some("empty_answer");
        }
        if response.finish_reason.as_deref() == Some("length") {
            return Some("truncated");
        }
        if response.continue_turn.is_some() {
            return Some("already_continued");
        }
        if ctx.limits.remaining_model_calls() < MIN_REMAINING_MODEL_CALLS {
            return Some("model_call_budget");
        }
        None
    }
}

#[async_trait]
impl<C: Send + Sync> Middleware<(), C> for UnmetDeliverableMiddleware {
    fn name(&self) -> &str {
        "unmet_deliverable"
    }

    async fn before_model(
        &self,
        ctx: &mut RunContext<C>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> Result<()> {
        let Ok(mut runs) = self.runs.lock() else {
            return Ok(());
        };
        let run = runs.entry(ctx.instance_id()).or_default();
        if run.candidates.is_some() {
            return Ok(());
        }
        // The request is the current turn's user message: the last user
        // message on the first call, since the thread's history rides ahead
        // of it and the harness's own user-role injections are wrapped as
        // instructions and arrive on later calls.
        let text = request
            .messages
            .iter()
            .rev()
            .find(|message| {
                matches!(message, Message::User(_))
                    && !message.text().contains("<harness_instruction>")
            })
            .map(Message::text)
            .unwrap_or_default();
        run.candidates = Some(candidate_paths(&text));
        Ok(())
    }

    /// The clock-driven rungs: at half-time a requested path that still does
    /// not exist is pointed out on the tool result the model is about to
    /// read, and at 80% the note says to stop exploring and meet the stated
    /// limits. Each note is appended once per run.
    async fn after_tool(
        &self,
        ctx: &mut RunContext<C>,
        _state: &(),
        _invocation: &ToolInvocationIdentity,
        result: &mut TaToolResult,
    ) -> Result<()> {
        let Some(clock) = TurnClock::of(ctx, self.budget) else {
            return Ok(());
        };
        let Some(band) = clock.band() else {
            return Ok(());
        };
        let (candidates, half_noted, late_noted) = {
            let Ok(runs) = self.runs.lock() else {
                return Ok(());
            };
            match runs.get(&ctx.instance_id()) {
                Some(run) => (
                    run.candidates.clone().unwrap_or_default(),
                    run.half_noted,
                    run.late_noted,
                ),
                None => return Ok(()),
            }
        };
        if band >= LATE_BAND && !late_noted {
            // The late note supersedes the half-time one: a run whose first
            // observation is already past 80% must not get the half-time
            // note on its next tool result.
            if let Ok(mut runs) = self.runs.lock() {
                let run = runs.entry(ctx.instance_id()).or_default();
                run.late_noted = true;
                run.half_noted = true;
            }
            tracing::debug!(
                band,
                "[unmet_deliverable] late note: stop exploring, meet the stated limits"
            );
            append_note(result, &late_note(&clock));
            return Ok(());
        }
        if (HALF_TIME_BAND..LATE_BAND).contains(&band) && !half_noted {
            let missing = Self::missing(&candidates);
            if let Ok(mut runs) = self.runs.lock() {
                runs.entry(ctx.instance_id()).or_default().half_noted = true;
            }
            if !missing.is_empty() {
                tracing::debug!(
                    band,
                    missing = missing.len(),
                    "[unmet_deliverable] half-time note: requested paths still absent"
                );
                append_note(result, &half_time_note(&missing, &clock));
            }
        }
        Ok(())
    }

    async fn after_model(
        &self,
        ctx: &mut RunContext<C>,
        _state: &(),
        response: &mut ModelResponse,
    ) -> Result<()> {
        let candidates = {
            let Ok(runs) = self.runs.lock() else {
                return Ok(());
            };
            match runs.get(&ctx.instance_id()) {
                Some(run) if !run.fired => run.candidates.clone().unwrap_or_default(),
                _ => return Ok(()),
            }
        };
        if candidates.is_empty() {
            return Ok(());
        }
        if let Some(reason) = Self::skip_reason(ctx, response) {
            tracing::debug!(reason, "[unmet_deliverable] not holding this answer");
            return Ok(());
        }
        let missing = Self::missing(&candidates);
        if missing.is_empty() {
            return Ok(());
        }
        if let Ok(mut runs) = self.runs.lock() {
            runs.entry(ctx.instance_id()).or_default().fired = true;
        }
        tracing::debug!(
            missing = missing.len(),
            "[unmet_deliverable] holding the answer: a named output file was never written"
        );
        response.continue_turn = Some(notice(&missing));
        Ok(())
    }

    /// The turn is over: drop its state, whether it ended with an answer or an
    /// error, so nothing is kept for a run that will not be seen again.
    async fn after_agent(
        &self,
        ctx: &mut RunContext<C>,
        _state: &(),
        _run: &mut tinyagents_harness::middleware::AgentRun,
    ) -> Result<()> {
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(&ctx.instance_id());
        }
        Ok(())
    }

    async fn on_error(&self, ctx: &mut RunContext<C>, _error: &TinyAgentsError) -> Result<()> {
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(&ctx.instance_id());
        }
        Ok(())
    }
}

/// Install the check on `harness` on the same scope as the requirements check
/// (`verify_before_finish::applies`): root orchestrator turns only, since a
/// sub-agent answers to its parent and the parent's own request is the one
/// that names a file. The policy-level wall clock is handed over because the
/// middleware cannot read `RunPolicy` from the run context.
pub(crate) fn install<C: Send + Sync + 'static>(
    harness: &mut AgentHarness<(), C>,
    is_subagent: bool,
    agent_definition_id: Option<&str>,
) {
    if !crate::agent::tinyagents::verify_before_finish::applies(is_subagent, agent_definition_id) {
        tracing::debug!(
            is_subagent,
            agent = agent_definition_id.unwrap_or("<none>"),
            "[unmet_deliverable] not installed for this turn"
        );
        return;
    }
    let turn_budget = harness
        .policy()
        .limits
        .max_wall_clock_ms
        .map(std::time::Duration::from_millis);
    harness.push_middleware(Arc::new(UnmetDeliverableMiddleware::new(turn_budget)));
}

#[cfg(test)]
#[path = "unmet_deliverable_tests.rs"]
mod tests;
