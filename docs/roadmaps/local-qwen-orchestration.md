# Local Qwen Orchestration Roadmap

Status: active
Owner: OpenHuman host integration
Branch: `fix/local-qwen-history`
Baseline: `c49bfeecd5406d87c3727f744fcb988217c8dccd`
Current implementation head: `28c9357ace48811c9d671c59610cc3f9ffaaeebf`

Architecture decision update (2026-09-15): Phase 7 supersedes the earlier
decision to retain TinyAgents as the primary-chat loop. The live build proved
that correct Qwen parsing and narrower schemas are insufficient while ordinary
chat, tool-assisted requests, and autonomous work share one loop and while
managed, metered capabilities are enabled without an explicit cost policy.
Phases 0-6 remain historical evidence and regression requirements; Phase 7 is
the authoritative design for the next build.

## Historical objective (Phases 0-6; superseded by Phase 7)

Make `lmstudio:qwen38-openhuman` a reliable primary orchestrator without
replacing OpenHuman, TinyAgents, LM Studio, llama.cpp, the model, or the current
tool-call mode. Ordinary requests must expose only the capabilities they need,
permanent backend failures must stop immediately, and the context gauge must
show actual primary-model occupancy rather than cumulative turn traffic.

Qwen-Agent is a compatibility oracle and benchmark only. It is not a shipped
runtime dependency.

## Verified baseline

- LM Studio reports `qwen38-openhuman` as 27,320,697,856 parameters, Q5_K,
  `n_ctx=131072`, and `n_ctx_train=262144`.
- OpenHuman now resolves the alias to 131,072 before LM Studio's 8,192 fallback.
- The live local profile reports `supports_native_tools=false`; this work keeps
  the existing prompt-guided mode unchanged.
- A fresh orchestrator turn builds a 72,455-character system prompt.
- The first simple call reported 22,528 input tokens.
- The broad registry contains 238 tools; OpenHuman precomputation currently
  registers 28 for the orchestrator.
- TinyAgents emits a `ToolsFiltered` event showing the other 210 tools were
  withheld, but `OpenHumanToolExposureShadowMiddleware` does not enforce its
  own result on the live request.
- The image-generation example finished with 76,198 cumulative input tokens,
  71,654 cached-input tokens, and three parent iterations. These are turn
  totals, not proof that one request occupied 76,198 tokens.
- The UI's `lastTurnContextUsed` adds all primary model calls in a turn. It is
  therefore throughput, not current context occupancy, despite being presented
  as the latter.
- GMI image generation returned HTTP 400 `Insufficient balance`. This is a
  permanent account-level failure and cannot be repaired by retrying during the
  turn.

## Constraints

This feature must not change:

- model weights, GGUF, quantization, or chat template;
- llama.cpp or LM Studio launch configuration;
- the 131,072 runtime context;
- `http://127.0.0.1:1234/v1`;
- `lmstudio:qwen38-openhuman`;
- prompt-guided/native tool-mode selection;
- global LM Studio fallback values;
- approval, sandbox, path, credential, or autonomy policy;
- cloud behavior for unrelated providers.

No new orchestration framework is allowed unless the existing TinyAgents
surfaces demonstrably cannot meet an acceptance criterion. Python and
Qwen-Agent must not enter the shipped process.

## Historical architecture decision (Phases 0-6; superseded by Phase 7)

OpenHuman remains the policy and execution host. TinyAgents remains the Rust
agent loop. LM Studio/llama.cpp remains the inference and Qwen-format adapter.

The production path becomes:

1. Resolve the agent's broad capability ceiling using existing security,
   profile, channel, MCP, and parent-agent restrictions.
2. Build a turn-stable exposure plan from the latest user request and current
   task state.
3. Intersect the exposure plan with the broad ceiling. Selection may only
   narrow; it can never grant a capability.
4. Register and advertise only the selected tools.
5. Enforce the same set in TinyAgents immediately before model and tool calls.
6. Load detailed skill instructions and packed tool schemas only after the
   model chooses `use_skill`.
7. Return bounded tool results and stop on terminal failures.

## Reference architecture sources

This roadmap borrows established behavior instead of introducing a new agent
framework:

- Qwen-Agent is the compatibility oracle for Qwen function schemas, canonical
  `{name, arguments}` calls, ordered tool results, and multi-step/parallel
  calls: <https://github.com/QwenLM/Qwen-Agent>.
- goose is the reference for a small provider-independent Rust execution loop
  that surfaces tool errors, revises context, and continues until a final model
  response: <https://github.com/aaif-goose/goose/blob/main/documentation/docs/goose-architecture/goose-architecture.md>.
- AnythingLLM is the reference for selecting a small relevant tool set before
  inference: <https://github.com/Mintplex-Labs/anything-llm-docs/blob/main/pages/agent/setup.mdx>.
- LangGraph is the reference for explicit state transitions, durable state,
  and idempotent re-execution: <https://langchain-ai.github.io/langgraph/reference/>.
- AutoGen is the reference for explicit success, failure, timeout, usage, and
  handoff termination conditions:
  <https://microsoft.github.io/autogen/stable/user-guide/agentchat-user-guide/tutorial/termination.html>.

Only OpenHuman-specific policy is implemented locally: mapping an authorized
user intent and runtime state to OpenHuman tool capabilities, failure classes,
fallbacks, and completion conditions.

## Exposure policy

Selection is deterministic, local, explainable, and stable for the duration of
one parent turn so prompt-prefix caching remains effective.

### Always-visible recovery tools

- `ask_user_clarification`
- `use_skill`
- `tool_search` when that deferred-tool recovery surface is present

### State-required tools

- Goal tools only while the thread has an active goal.
- Plan/task-board tools only while a plan or task board is active or the user
  explicitly requests planning.
- Subagent lifecycle tools only while a child exists or the request is selected
  for delegation.
- Approval/recovery tools only when their corresponding state is active.

### Prompt-selected tools

Use the existing `tinyagents_harness::tool::rank_tools_by_prompt` implementation
against the already-authorized candidate set. Select at most eight ordinary
tools, in addition to state-required and recovery tools. Categories with a
strong explicit intent, such as web lookup, repository editing, memory, media,
or scheduling, receive their matching tools even when lexical ranking is thin.

A zero-match conversational request gets only recovery tools. A thin actionable
match gets recovery tools plus the best matching category, never the entire
catalogue. The previous "thin match means expose everything" behavior is not a
safe fallback for the primary orchestrator.

Every selected tool must be both advertised and callable. Every unselected tool
must be neither advertised nor callable. Unknown-tool recovery may explain what
is available but must not execute a hidden tool.

## Prompt policy

- Keep the current identity, security, workspace, and agent-role contracts.
- Record the character and estimated-token contribution of every prompt section
  before deleting or rewriting prose.
- Remove only duplicated instructions or instructions for capabilities absent
  from the turn's exposure plan.
- Do not render prose tool instructions when the same tool schema already
  carries the contract required by prompt-guided mode.
- Keep the initial no-tool request at or below 20,000 provider-reported input
  tokens. Keep an ordinary single-tool request at or below 24,000 on its first
  model call.
- Keep installed-skill descriptions sanitized and capped, and keep detailed
  packed-tool instructions behind the existing session-scoped `use_skill`
  lookup rather than injecting them into every model call.

## Context and usage telemetry

The system must expose two different concepts instead of conflating them:

- Turn traffic: the sum of input, cached input, output, and cost across all
  model calls and subagents. This remains the billing/activity breakdown.
- Context occupancy: input plus output tokens for the most recent primary-model
  call, or the maximum primary-model call occupancy in the turn if the provider
  cannot report the final call separately. Subagent tokens are excluded.

The context-window pill uses context occupancy. Its tooltip may show cumulative
turn/session traffic, but labels must say so. Occupancy must never be calculated
by summing repeated cached prefixes across model calls.

## Terminal failure policy

Classify account quota/balance exhaustion, invalid credentials, forbidden
provider configuration, and unsupported provider/model requests as terminal.
A terminal tool or delegated-inference failure:

1. records a stable failure class and bounded root cause;
2. halts that run after the first observed failure;
3. produces one actionable response;
4. does not suggest an automatic retry;
5. does not silently switch to another paid tool sharing the failed backend.

Timeouts, connection resets, rate limits with retry guidance, and 5xx responses
remain recoverable under the existing bounded retry policy. Independent tool
failures remain governed by the current no-progress ladder.

## Delivery phases and gates

### Phase 0: specification and baseline

- [x] Record constraints, architecture, acceptance criteria, and baseline.
- [x] Add reproducible prompt/tool fixtures and capture baseline logs.

Gate: baseline evidence distinguishes per-call occupancy from cumulative turn
traffic and records prompt sections and visible tool names.

### Phase 1: trustworthy telemetry

- [x] Add primary-call occupancy to the core turn-usage contract.
- [x] Preserve cumulative parent/subagent accounting separately.
- [x] Correct the context-window pill and labels.
- [x] Add Rust and frontend regression tests.

Gate: a three-call turn cannot make the gauge report the sum of all three
prompt prefixes; subagent usage cannot affect primary occupancy.

### Phase 2: authoritative exposure

- [x] Introduce a pure, unit-tested primary-agent exposure planner.
- [x] Reuse TinyAgents ranking and contextual selection.
- [x] Apply the plan before tool registration and on the live model request.
- [x] Install an execution-time allow guard for defense in depth.
- [x] Retire shadow-only comparison after parity tests pass.
- [x] Preserve stable tool order and prompt-cache layout within a turn.

Gate: all exposure/security tests pass, no hidden tool is callable, and the
four representative prompts expose only their expected capability families.

### Phase 3: prompt and skill payload

- [x] Add per-section prompt-size observability without logging prompt text.
- [x] Remove the duplicate frozen P-Format catalogue; TinyAgents remains the
  sole live prompt-guided/native tool-protocol owner.
- [x] Keep detailed skill/tool instructions behind the existing scoped
  `use_skill` lookup and retain the existing 240-character description cap.
- [x] Verify prompt-guided Qwen tool-call parsing remains green.

Gate: first-call token targets are met without changing tool mode or weakening
instructions.

### Phase 4: terminal failures

- [x] Reuse the existing terminal backend classifier rather than duplicate it.
- [x] Verify delegated insufficient-balance runs halt on the first occurrence.
- [x] Keep untrusted output from falsely triggering a terminal halt.
- [x] Verify 5xx remains recoverable under the bounded retry policy.

Gate: permanent failures make one attempt; recoverable failures retain bounded
headroom.

### Phase 5: evaluation and release

- [x] Run focused Rust and frontend tests.
- [x] Run formatting, clippy for changed targets, and repository checks.
- [x] A/B the same fixtures through direct LM Studio and OpenHuman.
- [x] Run live greeting, news, image-failure, and repository-tool scenarios.
- [x] Rebuild the Windows application, NSIS installer, and MSI with the MSVC
  toolchain.
- [x] Review the exact diff, commit, push, and record artifact hashes/paths.

Gate: all acceptance criteria pass and the source tree is clean.

### Phase 6: contract-driven Qwen orchestration (next build)

The Phase 5 live gate is reopened. Live testing after
`f6504d24248d85d837b13ae47c985022ab760257` found four contract violations:

- an image-retrieval request was routed to image generation instead of search;
- `spawn_async_subagent` instructed the model to call `wait_subagent`, although
  that tool was not callable by the parent;
- a later parent turn instructed Qwen to call `memory_recall`, although the
  live allowlist omitted it;
- Qwen emitted `<tool_call>{"arguments":{"url":...}}</tool_call>` without a
  tool name, which was rendered to the user instead of being executed or
  corrected.

#### Single orchestration contract

Add one immutable per-turn contract derived from the already-authorized tool
ceiling. It must be the sole input to:

1. prompt capability instructions;
2. advertised model tool schemas;
3. execution-time allowlist enforcement;
4. retry/fallback behavior; and
5. completion validation.

The contract contains:

- `intent_family`: conversation, web/news, image retrieval, image generation,
  repository, memory, scheduling, or delegation;
- `allowed_tools`: stable ordered names intersected with the security ceiling;
- `required_state_tools`: tools justified by live goal, memory, approval, or
  child-agent state;
- `fallbacks`: ordered alternatives that do not change modality or paid/local
  boundaries without user authorization;
- `terminal_failures`: typed failures that cannot improve during this run;
- `completion`: final text, explicit handoff, or one actionable terminal error;
- `limits`: maximum calls, identical retries, result bytes, and correction
  attempts.

Prompt text must be generated from this contract. A tool absent from
`allowed_tools` must not be named as an instruction, advertised to the model,
or accepted for execution. This invariant is checked immediately before every
model call and every tool call.

#### Intent and capability rules

| Intent | Allowed capability | Forbidden implicit fallback | Completion |
| --- | --- | --- | --- |
| `find/show/get an image from the internet` | web/image search and retrieval | image generation | retrieved result or clear retrieval failure |
| `generate/create/draw an image` | image generation | paid web/media service not already authorized | generated artifact or one actionable failure |
| current news/web lookup | search then fetch | workspace, memory, media, delegation | bounded sourced answer |
| memory request | memory only when module health and access allow it | pretending recall occurred | recalled result or clear unavailable state |
| repository work | workspace/read/edit/shell under existing policy | web/media unless explicitly requested | verified change or blocker |
| delegated work | spawn/continue and automatic result delivery | unavailable polling tool | delivered result or terminal child failure |

Follow-up turns such as `try a different site` inherit the active intent family
but not stale tool results, failed provider choices, or hidden capabilities.

#### Qwen protocol boundary

Prefer native structured provider `tool_calls`. For prompt-guided text calls,
apply Qwen-Agent-compatible parsing only after the complete stream is assembled.

- Accept canonical `{ "name": string, "arguments": object }` calls.
- Retain the current narrow bare-name repair for `qwen38-openhuman`.
- A missing name may be inferred only when exactly one allowed tool matches the
  intent family and validates the argument object against its schema.
- Otherwise return one bounded validation correction to the model. A second
  invalid call ends with one clear error.
- Never execute an unadvertised tool, invent missing arguments, or infer a
  destructive tool.
- Never persist or render raw `<tool_call>` markup as assistant text.
- Preserve call IDs, call order, reasoning metadata required by the provider,
  and one result for every accepted call.

Tool-call dialect parsing remains owned by TinyAgents. OpenHuman supplies the
turn contract, authorized schemas, and model-specific compatibility decision;
it must not grow a second general parser or agent loop.

#### Lifecycle and failure rules

- `spawn_async_subagent` must not instruct the parent to call a tool outside
  its contract. Use the existing automatic result-delivery path; expose a wait
  tool only if it becomes an explicit, state-required capability.
- Memory instructions are emitted only when `memory_recall` is callable. A
  failed or unloaded memory module produces one typed unavailable result.
- Terminal billing, quota, authentication, configuration, and unsupported
  capability failures propagate across parent and child boundaries and stop
  that failing route after one attempt.
- Retry only typed transient failures. At most one retry is allowed unless the
  tool supplies explicit retry guidance.
- A successful non-terminal tool result returns to the model for a final
  response. A turn may not finish with only a URL, raw call, or internal status.
- Web/RSS/HTML results receive a deterministic byte/item cap before optional
  model summarization. Summarizer failure falls back to bounded parsed content,
  never the complete raw document.
- Side-effecting calls carry a stable idempotency key for the turn and call so
  stream replay or resume cannot duplicate the action.

#### Ownership scope

Expected OpenHuman touch points are limited to:

- `crates/openhuman-core/src/agent/harness/primary_tool_exposure.rs` for intent
  families and authorized tool selection;
- `crates/openhuman-core/src/agent/harness/session/turn/graph.rs` for live
  state-required capabilities and the per-turn contract;
- `crates/openhuman-core/src/agent/tinyagents/middleware/tool_exposure.rs` for
  prompt/advertisement/execution parity;
- `crates/openhuman-core/src/agent/orchestration/tools/spawn_async_subagent.rs`
  for truthful async lifecycle instructions;
- `crates/openhuman-core/src/agent/tinyagents/middleware/loop_guards.rs` and
  `repeated_failure.rs` for typed parent/child failure propagation;
- the existing TinyAgents tool-call adapter seam for bounded Qwen correction;
- existing tool-result artifact/context code for deterministic result limits.

No frontend policy, second tool registry, Python sidecar, new agent framework,
or provider-specific copy of OpenHuman's security rules is in scope.

#### Delivery order and gates

1. **Contract parity:** create the per-turn contract and prove prompt,
   advertisement, and execution contain the same tool names.
2. **Intent routing:** separate image retrieval from generation and make
   follow-up intent inheritance explicit.
3. **Protocol recovery:** handle canonical, safely inferable, ambiguous,
   unadvertised, malformed, streamed, and parallel Qwen calls.
4. **Lifecycle:** remove unavailable async instructions, gate memory on health,
   propagate terminal child failures, and enforce final-response completion.
5. **Result control:** bound web/media results and provide a non-LLM fallback
   when summarization fails.
6. **Release:** run focused suites, live scenarios, MSVC packaging, exact diff
   review, clean-tree verification, and publish artifact paths/hashes.

Each gate must pass before the next behavior is integrated. Existing context,
Windows loading, approval, sandbox, and security regressions remain required.

#### Phase 6 acceptance scenarios

- `hey`: one final response, no ordinary task tool, no raw protocol markup.
- `show me the top three Google News headlines`: web-only route, at most three
  primary calls, bounded results, three sourced headlines, and final text.
- `get me a pic of a blonde Asian woman from the internet; don't generate`:
  retrieval tools only, no image agent or generation call, and a returned
  result rather than a bare URL/tool call.
- `try a different site`: preserves image-retrieval intent and excludes the
  previously failed source without switching modalities.
- `generate a portrait ...` with `Insufficient balance`: exactly one media
  attempt and one actionable final response.
- memory enabled and healthy: `memory_recall` is both advertised and callable;
  memory unavailable: it is neither instructed nor advertised and no
  `not on the allowlist` failure occurs.
- async delegation: no unavailable `wait_subagent` instruction or call; child
  completion is delivered once.
- missing-name Qwen call: execute only when schema/intent matching yields one
  safe candidate; otherwise perform one correction and finish clearly.
- malformed, partial, or unclosed streamed call: no execution and no raw tag
  appears in persisted history or the UI.

#### Phase 6 measurable release criteria

- zero prompt/advertisement/allowlist name mismatches in unit and live logs;
- zero raw `<tool_call>` leaks across the fixture suite;
- zero retries after a terminal failure;
- no more than one correction attempt for an invalid model call;
- one execution per accepted call ID, including resume/replay tests;
- greeting: one primary call; news and image retrieval: at most three primary
  calls; media quota failure: one media call;
- no user-message trim in the representative scenarios;
- the existing 20,000/24,000 first-call input targets remain satisfied;
- Windows executable, NSIS, and MSI build through the repository's MSVC path;
- live logs show model context `131072`, selected intent/tool names, typed stop
  reason, final-response completion, and no false 8K budgeting.

Gate: every Phase 6 scenario passes against deterministic fixtures and the live
`lmstudio:qwen38-openhuman` route, all relevant existing tests remain green,
the exact diff is reviewed, and the source tree is clean.

### Phase 7: replace the primary-chat orchestration layer

#### Decision

Replace the primary interactive-chat control loop with a pinned, attributed
Goose Agent state-machine integration. Do not write another general-purpose
agent loop. Use Qwen-Agent as the protocol oracle for the local-Qwen adapter,
and use Smolagents only as a behavioral reference for explicit completion and
small, single-action steps.

The imported runtime is the orchestration mechanism, not OpenHuman's product
policy. OpenHuman continues to own its tool implementations, security policy,
approvals, workspace boundaries, persistence, UI events, and the decision to
permit metered services.

Reference snapshots used for the design:

- Goose `goose-agent`, commit
  `53672c3f14bbf83959cea3e6fe0132a2e8b800af`, Apache-2.0:
  <https://github.com/block/goose/tree/53672c3f14bbf83959cea3e6fe0132a2e8b800af/crates/goose-agent>
- Qwen-Agent function-calling adapter, commit
  `31a4d36d123688581a9e9744427272b33ce940e0`, Apache-2.0:
  <https://github.com/QwenLM/Qwen-Agent/tree/31a4d36d123688581a9e9744427272b33ce940e0/qwen_agent/llm>
- Smolagents multi-step agent and default web tools, commit
  `30bb1161095dbae2271e6bc3cc4c219cc3897a57`, Apache-2.0:
  <https://github.com/huggingface/smolagents/tree/30bb1161095dbae2271e6bc3cc4c219cc3897a57/src/smolagents>

Any copied source must retain its license header and be accompanied by the
upstream license and a source/commit notice. No code is to be copied from
community posts or non-compatible projects.

#### Why the current architecture is rejected

The shipped default is a hybrid cloud agent even when inference is local:

- empty primary-chat visibility means the whole configured tool surface;
- search defaults to `managed`, whose canonical tool posts to the TinyHumans
  `/agent-integrations/parallel/search` backend;
- `web_search_tool` is a default-on family;
- signing in is enough to construct backend-billed media-generation tools;
- browser automation is disabled by default;
- tool schemas do not carry availability, backend class, monetary boundary,
  result modality, or fallback priority;
- the Phase 2/6 lexical planner returns an unordered set and cannot express
  route order or cost boundaries;
- every user message enters the same iterative agent loop, including greetings
  and questions that require no action.

That design is valid for a managed-service product, but it does not satisfy a
local-first assistant contract. The Phase 7 build must make the distinction
explicit in executable types rather than prompt prose.

#### User-visible execution modes

Every primary turn resolves exactly one mode before any model call. Resolution
is deterministic and local; it must not spend another model call on routing.

| Mode | Purpose | Model-call rule | Tool rule |
| --- | --- | --- | --- |
| `chat` | explanation, conversation, transformation of supplied text, or a question answerable from the model/context | exactly one call | no task tools; no memory recall; no delegation |
| `assist` | a bounded request requiring current data or one/few concrete actions | model, one action at a time, final model response; default maximum 3 calls | only the ordered capabilities selected for this request |
| `agent` | explicitly autonomous, multi-step work with verification, persistent goal, scheduling, or delegation | Goose state machine until completion/pause; configurable bounded maximum, default 12 | capability plan plus live state-required tools |

Mode resolution precedence:

1. An explicit user/UI override (`chat`, `assist`, or `agent`) wins.
2. A live resumable agent checkpoint remains `agent` unless the user cancels it.
3. A request explicitly asking to execute, build, edit, browse, fetch current
   information, schedule, remember, delegate, monitor, or continue an action is
   `assist` or `agent` according to whether it requires durable/multi-step work.
4. Everything else is `chat`.
5. Ambiguity defaults downward: `chat` before `assist`, and `assist` before
   `agent`. The system must never escalate merely because a tool exists.

The initial implementation may expose the resolved mode in logs and diagnostics
without adding a new UI control. The architecture must nevertheless accept an
explicit override so a later UI selector does not require another loop rewrite.

#### Capability contract

Replace name-substring matching with one typed descriptor per registered tool:

```text
ToolCapability {
  name
  operations[]          // answer, search, fetch, navigate, read, write,
                        // execute, generate, remember, schedule, delegate
  modalities[]          // text, web-page, image, audio, video, file
  backend               // local, local-browser, direct-network,
                        // user-byok, openhuman-managed
  monetary_boundary     // none, user's external account, TinyHumans balance
  side_effect           // none, local reversible, external reversible,
                        // external irreversible
  availability          // ready, disabled, unhealthy, missing-credential
  permission_level
  priority
}
```

Registration order is stable and preserved. The per-turn plan is an ordered
`Vec<ToolRoute>`, not a `HashSet`. A derived set may be used only for constant-
time enforcement. The ordered route is authoritative for prompt order,
fallbacks, diagnostics, and tests.

A capability is model-visible only when all of these are true:

1. compiled and registered;
2. enabled by runtime and user settings;
3. healthy enough to execute now;
4. allowed by the session/profile/channel/security ceiling;
5. relevant to the resolved turn mode and requested operation;
6. inside the turn's monetary boundary.

Tool descriptions must state what the operation returns and whether it causes
an external effect. Cost policy must never depend on whether the model notices
phrases such as “billed by the backend” inside a schema.

#### Monetary and provider policy

Default policy is local-first and no-surprise-spend:

1. `local` and `local-browser` routes;
2. `direct-network` routes that do not use a metered API;
3. explicitly configured user-BYOK routes;
4. TinyHumans-managed or other metered routes only after explicit opt-in.

Signing into OpenHuman does not constitute permission to spend. Selecting a
specific metered search engine or enabling “Allow metered agent tools” does.
The permission is persisted as an explicit setting, is visible in diagnostics,
and can be overridden downward per turn. A model may not grant or broaden it.

When a route reports insufficient balance, quota exhaustion, invalid
credentials, or a missing provider, mark that route unavailable for the rest
of the turn. Continue only to the next already-authorized route. Never switch
from local/free to metered, or from retrieval to generation, as fallback.

#### Required intent policies

These policies are OpenHuman customization and must be represented as data plus
tests, not as an expanding natural-language prompt:

| Request | Mode | Ordered route | Prohibited behavior | Completion |
| --- | --- | --- | --- | --- |
| greeting/general question | `chat` | none | memory, goals, skills, delegation, retries | one answer |
| current news/general web search | `assist` | configured local-browser search; configured non-metered search; opted-in BYOK; opted-in managed search | workspace/media/delegation and silent metered fallback | sourced answer, not raw feed |
| known URL | `assist` | `web_fetch`; browser if page requires interaction | search API before trying supplied URL | answer derived from bounded page content |
| find/show an existing image | `assist` | image-capable browser/search, then fetch/validate result | image generation and bare navigation-only completion | rendered image result plus source |
| generate/edit an image | `assist` | configured local generator, then explicitly opted-in remote generator | treating search as generation or spending without opt-in | produced artifact or capability-unavailable answer |
| explicit memory request | `assist` | healthy memory tool only | recalling on unrelated turns; advertising an unhealthy module | recalled/stored result or one clear unavailable answer |
| repository change | `agent` | workspace read/edit/test tools under existing policy | web/media unless requested | verified diff or blocker |
| scheduled/monitored task | `agent` | scheduler plus task-required tools | pretending a one-shot answer created a schedule | durable schedule identifier/status |

If no allowed route exists, the model receives no fake substitute. OpenHuman
returns a deterministic capability-unavailable observation that names the
missing configuration without claiming work occurred.

#### Goose state-machine integration

Vendor the pinned `goose-agent` crate and the minimum compatible
`goose-provider-types` surface. Do not import Goose Desktop, Goose providers,
its UI, recipes, or its tool implementations. First attempt a direct vendored
crate integration. If dependency versions conflict, adapt types at the boundary;
do not rewrite the state machine. Any decision to port source into an
OpenHuman-owned module instead requires an ADR showing why the pinned crate
cannot compile and mapping every retained Goose invariant.

The OpenHuman primary turn is assembled as an ordered operation list:

1. load the persisted conversation/checkpoint;
2. resolve mode and immutable capability contract;
3. apply context compaction when required;
4. prepare one provider request from the current state;
5. normalize and validate the complete provider response;
6. accept final text when it satisfies the completion contract;
7. validate exactly one next tool action for local Qwen;
8. enforce capability, monetary, approval, and security boundaries;
9. execute and persist exactly one typed observation;
10. re-enter from persisted state;
11. yield on final answer, clarification, approval, cancellation, terminal
    capability failure, or the mode-specific call ceiling.

The state machine must reload/rederive behavior from persisted conversation
state between passes. In-memory retry counters alone are insufficient because
the desktop app supports interruption and resume.

Adapters translate mechanically between Goose conversation effects/events and
OpenHuman messages/progress events. OpenHuman policy remains outside those
adapters. Existing approvals, sandbox enforcement, action-directory checks,
and event semantics are mandatory and cannot be weakened for compatibility.

#### Qwen protocol adapter

The provider boundary for `lmstudio:qwen38-openhuman` follows Qwen-Agent's
tested conversation invariants while preserving the existing prompt-guided
mode and chat template:

- assemble the complete stream before prompt-guided call parsing;
- prefer native structured `tool_calls` if the provider supplies them;
- accept canonical `{name, arguments}` and preserve call IDs/order;
- serialize every accepted call followed by exactly one matching result;
- expose at most one action per Qwen inference step even if the text contains
  several calls; remaining calls are discarded and replanned from fresh state;
- stop generation/parsing at tool-result/return boundaries equivalent to
  Qwen-Agent's `FN_RESULT` and `FN_EXIT` semantics;
- keep reasoning text separate from user-visible answer text;
- never render raw `<tool_call>` markup;
- allow one schema-informed correction for malformed, unknown, or ambiguous
  calls, then terminate clearly;
- never infer a missing destructive tool or synthesize missing arguments.

This adapter is a provider dialect adapter, not a second agent loop.

#### Completion and loop policy

Every turn has a machine-checkable completion contract:

- `chat`: non-empty final assistant text;
- news/search: final text containing the requested bounded result and sources;
- image retrieval: at least one validated image result/source rendered to the
  client, not merely a search-results page URL;
- artifact generation: an existing artifact path/URL plus final explanation;
- repository mutation: recorded change plus requested verification status;
- scheduling: persisted schedule identifier and state;
- clarification/approval: explicit yielded question or approval request.

A successful tool observation is not completion unless the contract says the
tool result itself is the product. After an informational tool call, the state
machine normally performs one final model call. It may use a deterministic
renderer instead when the tool returns a complete typed result and model
summarization is unnecessary or unavailable.

Loop guards are typed and stateful:

- identical call signature twice without changed relevant state: stop;
- same typed failure twice: stop, unless retry guidance explicitly authorizes
  one delayed retry;
- unavailable tool request: one correction, then stop;
- no progress across two state-machine passes: stop;
- call ceiling: yield a resumable checkpoint only for `agent`; `chat` and
  `assist` return a bounded failure rather than pretending success.

#### Memory behavior

Memory is not a universal recovery tool. It enters the capability contract only
when the user requests remembering/recalling, a live agent task explicitly
requires prior durable state, or a configured product policy requests bounded
context retrieval. Module readiness is checked before advertisement. A module
load failure removes memory tools from the turn and emits one diagnostic event;
it must not produce a failed Memory Recall card on unrelated conversations.

#### Migration and rollout

Add a core-owned configuration value:

```text
agent.orchestration_engine = "tinyagents" | "goose"
```

During development, default it to `tinyagents` for unrelated providers and
force/opt in `goose` for the exact `lmstudio:qwen38-openhuman` route. Before
release, make `goose` the primary interactive-chat default only after the full
fixture matrix passes. Keep the TinyAgents path for rollback through one
release; delete the Phase 2 lexical planner from the Goose path immediately so
there is only one authoritative plan.

Migration stages and gates:

1. **Vendor and license gate** — pinned source, license/notice, dependency
   audit, Windows compile, no runtime behavior change.
2. **Adapter gate** — OpenHuman messages, tools, approvals, progress, usage,
   cancellation, and persistence round-trip through the Goose state machine in
   deterministic tests.
3. **Mode gate** — direct chat bypass and `chat`/`assist`/`agent` resolution
   pass fixtures without an LLM routing call.
4. **Capability gate** — typed metadata, ordered routes, health filtering, and
   monetary enforcement pass; paid tools are absent by default.
5. **Qwen gate** — official-style transcript fixtures cover final text,
   canonical calls, streamed calls, malformed calls, hidden reasoning,
   correction, and resume.
6. **Use-case gate** — news, known URL, image retrieval, image generation,
   memory, repository edit, scheduling, failure fallback, and cancellation pass
   end-to-end against mock tools.
7. **Live gate** — the exact LM Studio route passes the required scenarios with
   logs and no user-message trimming.
8. **Release gate** — MSVC tests/build/package, diff and license review, clean
   tree, committed and pushed branch, artifact hashes recorded.

No later gate begins until the previous gate is reconciled against Git and its
literal verification command succeeds.

#### Phase 7 gate reconciliation

Implementation work after Gate 3 is decomposed in
[`local-qwen-orchestration-tasks.md`](local-qwen-orchestration-tasks.md). That
file is an execution registry only: this roadmap remains the sole source of
product requirements and gate acceptance. Workers receive one compact task
contract extracted by the orchestrator and do not read either document.

| Gate | Status | Reconciled evidence |
| --- | --- | --- |
| 1. Vendor and license | passed 2026-09-15 | The two upstream crate trees and Apache-2.0 license are byte-identical to Goose `53672c3f14bbf83959cea3e6fe0132a2e8b800af`; `rmcp` is fixed to the upstream lock's 3.3.0; the source notice and license are Windows bundle resources. `goose-agent` passed 17 tests. Locked MSVC checks passed for the vendored workspace, root workspace, and separate Tauri workspace. |
| 2. Adapter | passed 2026-09-15 | Direct `GooseTurnAdapter` integration drives the pinned `goose_agent::machine::StateMachine`; OpenHuman transcript/tool/RMCP/TinyInference conversion, existing security and approval, `AgentProgress`, per-call versus cumulative usage, cancellation observations, and optimistic per-step checkpoints passed 10 deterministic network-free tests. A failed accepted-action commit prevents execution; approval waits only after acceptance is durable; resume does not duplicate the effect or observation; the typed primary-call ceiling yields a resumable checkpoint. `cargo check --manifest-path crates/openhuman-core/Cargo.toml --lib` passed on MSVC. |
| 3. Mode | passed 2026-09-15 | Typed deterministic `chat`/`assist`/`agent` resolution and explicit RPC override run before the provider. The exact local-Qwen binding opts into Goose through persisted `agent.orchestration_engine`, while all unrelated providers/entrypoints and the explicit rollback setting retain TinyAgents. Direct chat bypasses `Agent::turn`, advertises zero tools, and made exactly one call for greeting/general-question fixtures. Assist and agent fixtures dispatch through the vendored Goose machine with mode ceilings of 3/12. Workspace-owned atomic checkpoints and effect claims survive fresh store handles. The two literal MSVC suites passed 6 mode/checkpoint tests and 3 dispatch tests; the Goose adapter suite passed 10 tests. |
| 4. Capability | passed 2026-09-16 | Typed intent, exact-name catalog, deterministic local/browser/direct/BYOK/managed planning, persisted managed-metered opt-in with downward turn override, ordered shared Goose tool snapshots, pre-approval route enforcement, and deterministic unavailable output are integrated. Network-free MSVC suites passed 8 planner, 14 Goose, 3 primary-turn, and 9 Gate 4 acceptance tests; the root library check also passed. Authentication alone never enables managed spending. |
| 5. Qwen | passed 2026-09-16 | Native call precedence, canonical `{name, arguments}` parsing, single-action enforcement, hidden reasoning isolation, raw tag stripping, and pure schema-informed correction prompt generation are integrated for the exact local-Qwen route (`lmstudio:qwen38-openhuman`). Protocol correction count and terminal protocol failure persist across file-store handles; a second invalid response terminates cleanly with safe user text; every accepted action pairs with exactly one observation; and resume creates no duplicate effect, orphan, or raw persisted markup. Network-free MSVC suites passed 26 Qwen tests and 16 Goose tests (42 total in `agent::goose`); the root library check also passed. |
| 6. Use cases | passed 2026-09-16 | Machine-checkable completion contracts (chat, web, image retrieval, artifact, repository, scheduling, clarification, approval) and persisted loop guards (duplicate call signature, repeated failure, unavailable request, two no-progress passes, call ceiling) integrated with Goose. 20 acceptance tests across web/media, action/memory/repo, and state/interruption (G6-G, G6-H, G6-I) and 25 Goose runner integration tests pass on MSVC. Commits fa21987fd..a1d89a2cc. |
| 7. Live | passed 2026-09-16 | Reproducible live matrix runner (`scripts/debug/local-qwen-phase7-live.mjs`) unit-tested and executed against local LM Studio (`http://127.0.0.1:1234/v1`) with model `qwen38-openhuman` and context `131072`. All 12 scenarios passed: greetings_chat, factual_chat, news_search, known_url_fetch, image_retrieval_not_generation, disabled_metered_generation, metered_zero_balance_terminal, unrelated_chat_failed_memory, explicit_recall_failed_memory, repository_edit_and_verification, qwen_missing_name_recovery, and interruption_and_resume. No raw `<tool_call>` markup, no duplicate effects/orphans, zero unapproved paid requests, and explicit stop reasons recorded in `target/phase7-live/evidence.json` and `evidence.md`. Commits 13b46db62 and 861b6f5d0. |
| 8. Release | passed 2026-09-17 | Non-destructive MSVC validator `scripts/release/phase7-windows-evidence.ps1` (14 unit tests passing) verified `x86_64-pc-windows-msvc` toolchain (rejecting MinGW/GNU). Focused Gate 1–6 suites (80/80 passed), `cargo fmt --check`, `cargo clippy` (`-D warnings`), `pnpm compile`, `pnpm docs:check`, frontend dist build, release core build (`cargo build --release -p openhuman`), release desktop build (`cargo build --release --manifest-path crates/openhuman-app/Cargo.toml`), and Tauri packaging (`tauri bundle --bundles nsis,msi`) passed cleanly. `OpenHuman.exe`, NSIS installer, and MSI verified and smoke-tested. Absolute paths, byte lengths, and SHA-256 hashes recorded: OpenHuman.exe (128,168,960 bytes, SHA256: 3195b4d0d4b0dd8a2533b7f4944a7091b07303ea78fdde63ec851d4b2a473f39), OpenHuman_0.63.25_x64-setup.exe (28,382,812 bytes, SHA256: b0de695ea437e3a943c98f662b0f859606fc8eccdfc60ef2d6dfa36f1f2a1098), OpenHuman_0.63.25_x64_en-US.msi (42,934,272 bytes, SHA256: 73078fba7ce1ea2b57d9af3c4640b39eaa60f7397750d864458d2cb52407a330). |

#### Required automated coverage

At minimum, add deterministic tests for:

- all mode resolution rules and explicit overrides;
- zero tools and exactly one model call for ordinary chat;
- descriptor availability, security ceiling, stable ordering, and duplicate
  rejection;
- paid managed search/media absent by default, present only with explicit
  persisted opt-in, and impossible for the model to self-enable;
- browser/direct-network/BYOK/managed ordering;
- image retrieval never exposing generation;
- generation never exposing retrieval as a fake substitute;
- unhealthy memory never advertised or called;
- each accepted tool call paired with exactly one result across interruption,
  cancellation, and resume;
- one-action-per-step for local Qwen;
- malformed and raw-tag output correction/termination;
- terminal route failure followed by an already-authorized free fallback;
- no fallback crossing modality, permission, side-effect, or monetary
  boundaries;
- final-response completion after informational tools;
- deterministic completion without a summarizer when bounded structured output
  is already sufficient;
- cumulative usage remaining distinct from current context occupancy;
- the existing 131,072 alias and Windows module-loading regressions.

Tests must use mock providers/tools and make no real backend or third-party
requests.

#### Live acceptance matrix

Run each prompt in a fresh thread and record resolved mode, advertised ordered
tools, calls, final state, latest-call context occupancy, cumulative traffic,
wall time, and external endpoints contacted.

1. `hey` — `chat`, one call, zero tools, final answer.
2. `explain why the sky is blue` — `chat`, one call, zero tools.
3. `show me the top three Google News headlines` — `assist`, non-metered route
   unless metered search was explicitly enabled, sourced final answer, at most
   three calls.
4. supplied known URL — fetch first, bounded final answer.
5. `get me a picture ... from the internet; don't generate` — retrieval only,
   actual rendered result/source, no media generation.
6. `generate a portrait` with no local generator and metered tools disabled —
   zero paid requests and one clear unavailable response.
7. same generation request with explicit metered opt-in and zero balance — one
   request, route marked unavailable, one final actionable response.
8. unrelated chat with failed memory module — no memory tool/card/error.
9. explicit recall with failed memory module — one clear unavailable response.
10. repository edit/test — `agent`, approved workspace tools, verified diff.
11. malformed missing-name Qwen call — one correction maximum, no raw markup.
12. interruption/resume — no duplicated side effect or orphaned tool result.

For every scenario, logs must show context `131072`, no false 8K trim, no
unapproved managed endpoint, and an explicit stop/yield reason.

#### Build and release verification

Use the repository's Microsoft MSVC build path. Required evidence:

- focused Rust unit/integration suites;
- frontend tests for mode/cost presentation if UI is changed;
- formatting, changed-target clippy, typecheck, and repository architecture
  checks;
- release `OpenHuman.exe`, NSIS installer, and MSI;
- exact absolute paths, sizes, and SHA-256 hashes;
- clean `git status`, exact diff, commit IDs, and remote branch match;
- live log excerpts for every acceptance claim.

Do not ship a MinGW artifact or an executable missing its required DLLs.

#### Definition of done

Phase 7 is complete only when:

- ordinary OpenHuman chat behaves like chat, not an autonomous agent;
- assisted and agent turns run through the imported Goose state machine;
- local Qwen uses the tested Qwen protocol adapter;
- no metered search, media, or integration request can occur without explicit
  user authorization;
- capability availability and fallback order are executable typed data;
- all acceptance scenarios pass in mocks and on the specified live route;
- no raw tool markup, orphaned result, duplicate side effect, unrelated memory
  failure, or false completion remains;
- the Windows installer is rebuilt, installed/smoke-tested, committed, and
  pushed with reproducible evidence.

#### Explicit non-goals for Phase 7

- changing the model, GGUF, quantization, chat template, llama.cpp flags,
  endpoint, provider route, or 131,072 context;
- importing Goose's UI, providers, commercial services, or tools;
- silently enabling browser access to unrestricted domains;
- adding a local image model when none is configured;
- replacing OpenHuman's security, approval, persistence, or presentation
  layers;
- treating a larger prompt or more retries as an orchestration fix.

## Representative acceptance scenarios

### Conversation

Prompt: `hey`

- No task tool is called.
- At most the recovery tools are exposed.
- One model call completes the turn.
- First-call input is at most 20,000 tokens.

### News

Prompt: `show me the top three Google News headlines`

- Only web/recovery tools are exposed.
- No workspace, goal, subagent-management, media, or repository tool appears.
- The task completes in at most three primary model calls and one successful
  fetch/search path.
- No user message is trimmed.

### Media quota failure

Prompt: `generate a portrait of a blonde K-pop-inspired woman`

- Only media/recovery/delegation capabilities needed for the route are exposed.
- HTTP 400 `Insufficient balance` results in one generation attempt and one
  actionable final response.
- The run does not retry GMI or switch to another paid TinyHumans backend.

### Repository edit

Prompt: `change the README heading and run its focused test`

- Repository read/edit/shell tools are exposed.
- Media, web, memory-maintenance, scheduling, and unrelated integrations are
  absent.
- Existing approval and action-directory restrictions remain effective.

## Required automated coverage

- Exposure planning: zero-match chat, explicit category match, ambiguous match,
  state-required tools, stable ordering, top-K, parent ceiling, denylist, empty
  allowlist, and hidden-tool execution rejection.
- Prompt construction: unavailable capability prose omitted; required security
  and recovery contracts retained; deterministic section-size snapshot.
- Usage: last primary call versus cumulative calls, cached tokens, subagent
  subtraction, unknown context window, and resumed thread.
- Failures: terminal balance/quota/auth/config, recoverable timeout/rate-limit/
  5xx, parallel results, and delegated terminal inference.
- Existing context-resolution regression for `qwen38-openhuman` remains green.

## Rollout and rollback

Land telemetry with the behavior change. The planner only narrows the existing
authorized ceiling and logs counts and names, never prompt text or tool
arguments.

Rollback is reverting authoritative relevance narrowing while retaining the
correct 131,072 context resolver and Windows loader fix; the security ceiling
continues to apply independently.

## Historical non-goals (Phases 0-6; superseded by Phase 7)

- Replacing TinyAgents with Qwen-Agent, LangGraph, PydanticAI, or Smolagents.
- Changing the model, endpoint, context length, chat template, or tool mode.
- Adding local image generation.
- Fixing unrelated memory persistence, cloud billing, UI, or module issues.
- Claiming that prompt caching reduces context occupancy; it reduces repeated
  computation/cost, not the number of tokens the model attends to.

## Completion evidence

Completion requires the final report to include:

- exact source diff and commits;
- baseline versus final prompt characters, visible tools, per-call occupancy,
  cumulative turn traffic, model calls, and wall time for each scenario;
- focused and relevant suite results;
- MSVC build output, executable/installer paths, sizes, and SHA-256 hashes;
- live log lines proving 131,072 context, selected tool counts, no false trim,
  and terminal-failure behavior;
- remote branch verification and a clean worktree.
