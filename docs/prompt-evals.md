# Prompt evals

Two tiers answer two different questions. Do not cite one as evidence for the
other.

| | Tier 1 — routing | Tier 2 — comprehension |
|---|---|---|
| Question | Is the agent wired so a model *could* follow its prompt? | Does a real model actually follow it? |
| Where | `tests/agent_prompt_comprehension_e2e.rs` | `scripts/prompt-eval.sh` + `scripts/prompt-eval/cases.json` |
| Model | Scripted completions (no model) | The real backend model |
| Cost | Free, deterministic | Money per run, non-deterministic |
| CI | Runs with the Rust integration tests | **Never.** Refuses to run when `CI=true` |

## Tier 1 pins the script, not a model's judgment

Every tier-1 completion is scripted. When the `workflow_builder` case scripts
two `search_tool_catalog` calls and then `propose_workflow`, the test proves
the builder's belt carries both tools, that both calls resolve (no
`unknown tool`), that `flows_build` extracts the proposal, and that nothing in
the runtime turns two searches into three. It proves **nothing** about whether
a model, given that prompt, would propose instead of searching 27 times. That
was the 2026-09-17 incident: the right guidance was in the prompt, and the
model never reached it.

A green tier 1 is therefore not evidence of comprehension. Only tier 2 is.

The six cases cover the agents where mis-routing has cost something:
`workflow_builder` (must reach `propose_workflow`, never three consecutive
catalog searches), `orchestrator` (hands integration work off, never holds the
raw Composio or cron tools), `integrations_agent`, `scheduler_agent`, and the
two zero-belt agents `summarizer` and `trigger_triage` (advertise nothing).
Fleet-wide static coverage of all agents lives in the prompt tests under
`crates/openhuman-core/src/agent/registry/agents/`, not here.

```sh
cargo test -p openhuman --test agent_prompt_comprehension_e2e
```

## Tier 2

Build the binary first (`cargo build --bin openhuman-core`). There are two ways
to run it, and the difference between them matters.

**Hermetic (default).** Each case gets a fresh `mktemp -d` workspace
(`OPENHUMAN_WORKSPACE`). A fresh workspace has no keyring, so you must pass in
a credential. This is the path for headless hosts:

```sh
OPENHUMAN_BACKEND_SESSION_TOKEN=... scripts/prompt-eval.sh --case workflow-builder-news
# or OPENHUMAN_BACKEND_API_KEY=...; BACKEND_URL selects the backend
```

**Real workspace (`--real-workspace`).** Runs against the signed-in
`~/.openhuman`. The core reads its own keyring, so nobody handles a token.
The costs are:

- **Quit the desktop app first.** Only one process may own `~/.openhuman`,
  and the script refuses to start while the app or a core server is running.
- **It writes into the user's real account.** Every case leaves a thread
  behind, plus anything a case installed or scheduled (a workflow, a cron job
  from `orchestrator-reminder`). After the run, delete the test threads, remove
  any `cron_*` jobs and flows the cases created, and check that skills and MCP
  servers are back to where they started.
- **It is serial.** Only transcripts modified strictly after a case starts are
  scored for that case, and each temporary hermetic workspace is removed after
  its row is recorded. Real-workspace artifacts still require the account
  cleanup described above.

```sh
scripts/prompt-eval.sh --real-workspace --case workflow-builder-news
```

`--runs N` repeats every case N times, each in a fresh process (and a fresh
workspace when hermetic). One run is a sample, not a baseline. Every run is its
own row, keyed by case, timestamp, run index and model ids. Rows are never
averaged, so the variance stays visible.

Either way, each case runs in its own `openhuman-core call` subprocesses: one
to install the credential (hermetic only), one to run the agent
(`openhuman.flows_build` or `openhuman.agent_chat`), and one for the judge. A
process per case sidesteps the process-global model override and
`AlreadyRunning`, and needs no feature gate.

Scoring reads only artifacts the run already writes, in this order:

1. **Hard failure signals.** A breaker halt (the repeat/failure middlewares'
   halt log line), `[SUBAGENT_INCOMPLETE]` in a transcript, or `trail_off` /
   `capped` / `error` on the `flows_build` result. Any of these scores the run
   0: it was unproductive. The motivating incident would have tripped these.
2. **Did it do the thing.** Expected calls, forbidden calls and repeat caps,
   from the assistant `tool_calls` in `<ws>/**/session_raw/*.jsonl`.
3. **Cost.** Summed from each transcript's `_meta` (`input_tokens`,
   `output_tokens`, `cached_input_tokens`, `charged_amount_usd`), plus an
   `max_input_tokens` ceiling per case. The judge call is not included.
4. **Judge** (cases with `"judge": true`). The production close-verification
   rubric from `close_verification_prompt` in
   `agent/session_host/turn_checkpoint.rs`, copied into `cases.json` rather
   than widening that `pub(super)` function for an on-demand script. The
   verdict is read as `parse_close_verdict` reads it: the last standalone
   `ACCEPT`/`REJECT` token wins, so `UNACCEPTABLE` is not `ACCEPT`.

One row per case is appended to `target/prompt-eval-runs.jsonl`
(gitignored with `target/`), and the run prints the total USD.

**Latency.** `seconds` is the wall clock of the agent call itself, measured by
the harness around one `openhuman-core call` process. It covers boot (a no-op
call takes about 0.01–0.16 s), agent assembly, every model call and every tool
call. The judge call is excluded. It is total turn duration, which is what a
user waits for. Time to first token is not measured: `call` is non-streaming,
so the first token is never observable from outside. Transcript timestamps
cannot stand in for it, because `_meta.created`, `_meta.updated` and each
message's `ts` are all stamped when the file is written, not at turn
boundaries.

**Every row records the model id beside its cost.** A provider-side model
update silently rebaselines every score. Compare rows only when the model
ids match.

Tier 2 reads the on-disk transcripts, never Langfuse: Langfuse push is
disabled outside staging/dev by design, and only the web progress bridge
installs a collector at all. The run journal under
`<ws>/tinyagents_store/journal` is the natural enrichment if transcript
scoring proves too coarse.

### The cases

| Case | Surface | Checks | Writes |
|---|---|---|---|
| `workflow-builder-news` | workflow | reaches `propose_workflow`; ≤2 consecutive catalog searches; saves nothing | nothing |
| `orchestrator-reminder` | orchestration → scheduler | hands off through `schedule_task` | **a cron job**, remove it afterwards |
| `orchestrator-direct-answer` | orchestration | answers a trivial question without spawning | nothing |
| `composio-gmail-read` | composio | reads the latest Gmail subject via `delegate_to_integrations_agent`; never sends, deletes or reconnects | nothing |
| `skill-notion-read` | skills | lists Notion pages through `run_skill`; never installs a skill | nothing |
| `mcp-none-configured` | MCP, **error path** | with no MCP server configured, says so; never installs one, never fabricates results | nothing |
| `web-search-fact` | web search | one built-in `web_search_tool` lookup, not a `research` spawn | nothing |

(Rows are listed here by surface; `cases.json` holds them in run order.)

Prefer read-only cases. Against a real account every write is cleanup that
someone does by hand, so a case that must write lists what it leaves behind
in its `writes` field. Cases that depend on account state carry a
`precondition`, for example that Gmail is connected, the Notion skill is
installed, or no MCP server exists. Re-verify those before a run: the account
changes, and a case whose precondition no longer holds measures something
else. `mcp-none-configured` is an error-path case by design. Do not install an
MCP server to make MCP "testable"; that changes the baseline being measured.

Cases run in file order, by increasing account risk. Cases that touch
nothing run first, because they also validate the rig on real inference; the
only writing case (`orchestrator-reminder`) runs last.
**`composio-gmail-read` is disabled by default.** The toolkit-scoped `integrations_agent`
runs in text mode, so its calls may not reach `calls` in the same shape as
native tool calls. Until a real transcript has shown one landing in a form its
`GMAIL_*` forbids match, those forbids are unproven. They would fail to notice
a send, not prevent it. Run the earlier cases, inspect a real `calls` field,
and only then run it. Do not repair the matcher mid-run. The runner skips this
case unless `--allow-disabled` is explicitly supplied after matcher validation.

Each case with account preconditions checks them with a read-only RPC just
before every run (`precondition` in `cases.json`: gmail connected, notion skill
installed, no MCP server). The result, with its evidence, is recorded in the
row. A failed check skips the run, so a missing connection never reads as a
prompt failure. Those regexes are unverified against real output, so read
`precondition.evidence` in the first rows.

A call made through `use_skill` also counts as the packed tool it reaches, so
a forbidden packed tool is caught either way.

### Adding a case

Add an object to `cases` in `scripts/prompt-eval/cases.json`: `id`, `entry`
(`flows_build` or `agent_chat`), `message`, `expect_calls`, `forbid_calls`,
`max_consecutive` (`{tool: cap}`), `max_input_tokens`, `judge`, optional
`reply_regex`, `surface`, `writes`, `precondition`, and a `_why` naming the
failure it guards against. Keep the set small; every case costs
money on every run.
