# Epic 7964 final acceptance, web_chat ownership audit

Audited at the integrated revision on `epic/7964-channel-v2` after senamakel/openhuman#662. Line counts are informational.
Every remaining non-test file is OpenHuman policy or a host port: RPC and socket handlers, input trust, approvals, session checkout,
reply persistence, terminal policy, and adapters that call the TinyChannels module over TinyBus (`surface.rs`) and the TinyAgents turn runner (`runner.rs`, `turn_host.rs`).
No file keeps a second turn queue or a web-output event mapper: sequencing and the wire schema are in `tinychannels-bus`/the module, queue mechanics in `tinyagents_harness::turn_runner`.

| File | Lines | Role | Verdict |
| --- | ---: | --- | --- |
| `channel_event.rs` | 96 | Host-only routing envelope around TinyChannels' typed web event contract. | KEEP: host policy or port. |
| `egress_surface.rs` | 108 | Bridges [`DomainEvent::ExternalTransferPending`] onto the web-channel | KEEP: bridges an OpenHuman security event onto the web wire. |
| `event_bus.rs` | 699 | In-process `WebChannelEvent` broadcast bus, plus the `DomainEvent` | KEEP: process-wide broadcast of the TinyChannels wire type; the host owns delivery to Socket.IO, SSE, TUI. |
| `event_bus_channel.rs` | 36 | The process-wide `WebChannelEvent` broadcast channel and its publishers. | KEEP: host policy or port. |
| `ingress.rs` | 309 | What a message must clear before it may start, or join, a turn. | KEEP: host policy or port. |
| `journal_shadow.rs` | 177 | Journal-shadow parity: compare the live trace spans for a turn against the | KEEP: OpenHuman tracing parity check; no TinyChannels or TinyAgents equivalent. |
| `mod.rs` | 159 | Web/desktop channel host. | KEEP: host policy or port. |
| `ops.rs` | 58 | Web-channel chat dispatch: the entry points over the turn runner | KEEP: host policy or port. |
| `ops/budget_correlation.rs` | 175 | Per-thread inference-budget-exhaustion correlator (#3386). | KEEP: OpenHuman billing and budget policy. |
| `ops/channel_ops.rs` | 497 | Public RPC-facing operations for the web channel: sending a chat message, | KEEP: host policy or port. |
| `ops/start_chat.rs` | 575 | The primary and parallel turn entry point: validate the submission, hand it | KEEP: host policy or port. |
| `ops/state.rs` | 241 | Shared in-memory state for the web-channel turn dispatcher: the per-thread | KEEP: host policy or port. |
| `ops/system_turn.rs` | 111 | Host-authored turns on an existing conversation thread. | KEEP: host policy or port. |
| `ops/test_hooks.rs` | 205 | Process-global test hooks for `run_chat_task`: a forced-error override, a | KEEP: host policy or port. |
| `presentation.rs` | 295 | Presentation layer for web-channel chat responses: store a finished reply, | KEEP: host policy or port. |
| `proactive.rs` | 83 | Web-surface delivery for proactive messages. | KEEP: host policy or port. |
| `progress.rs` | 523 | One turn's progress, projected: the single consumer of a turn's | KEEP: host policy or port. |
| `progress_ledger.rs` | 24 | Host adapter for the owner-provided generic progress ledger recorder. | KEEP: host policy or port. |
| `progress_map.rs` | 8 | Compatibility view of the shared agent progress projection. | KEEP: 7-line re-export of the agent-domain progress constructors; no mapping logic left here. |
| `progress_subagent.rs` | 39 | Surface-specific logging around the shared sub-agent progress mapper. | KEEP: host policy or port. |
| `progress_trace.rs` | 90 | Structured tracing export for one bridged turn (#3886). | KEEP: host policy or port. |
| `reply_persistence.rs` | 97 | Durable storage for the reply a web-channel turn is about to announce. | KEEP: host policy or port. |
| `runner.rs` | 199 | The web chat's turn runner and the host state its ports share. | KEEP: host policy or port. |
| `schemas.rs` | 452 | `ControllerSchema`/`RegisteredController` definitions for the `channel` | KEEP: host policy or port. |
| `session.rs` | 624 | Builds and fingerprints the cached session `Agent`: target agent | KEEP: host policy or port. |
| `suggestions.rs` | 263 | Post-turn follow-up-suggestion generation for the web chat surface. | KEEP: product feature (follow-up suggestions) over the OpenHuman provider route. |
| `surface.rs` | 334 | The host's adapter between a web-chat turn and TinyChannels' web projection | KEEP: host policy or port. |
| `turn_activity.rs` | 149 | What one web-chat turn last did, kept for the reply that closes it when a | KEEP: host policy or port. |
| `turn_close.rs` | 359 | The reply that closes a turn a web-turn guard stopped. | KEEP: host policy or port. |
| `turn_events.rs` | 572 | The web chat's delivery ports for the turn runner: where a turn's progress | KEEP: host policy or port. |
| `turn_host.rs` | 651 | The web chat's [`TurnHost`]: session checkout and the agent run. | KEEP: host policy or port. |
| `turn_host_progress.rs` | 120 |  | KEEP: host policy or port. |
| `types.rs` | 163 | Shared state and RPC parameter types for the web chat turn: the cached | KEEP: host policy or port. |
| `voice_notes.rs` | 364 | Durable audio for in-app voice notes. | KEEP: durable audio lives in OpenHuman storage; candidate to move behind a TinyChannels media port later. |
| `web_errors.rs` | 22 | Turning a flattened inference/agent-loop error string into a stable, | KEEP: host policy or port. |
| `web_errors/backend_error_code.rs` | 118 | Classifying a managed-backend error by its stable `errorCode` (#870) — | KEEP: host policy or port. |
| `web_errors/budget.rs` | 46 | Detection and user-facing accessors for OpenHuman's own budget/quota limits — | KEEP: host policy or port. |
| `web_errors/classify.rs` | 411 | The chat-error classification ladder: turning a flattened inference error | KEEP: host policy or port. |
| `web_errors/retry.rs` | 42 | Formatting the "retry after" hint for users, and telling a retryable 429 | KEEP: host policy or port. |
| `web_errors/timeout.rs` | 77 | The synthetic marker a wedged web turn raises when the wall-clock backstop | KEEP: host policy or port. |

40 non-test Rust files, 9571 physical lines.
