# Inbound

The host side of the `tinyeventqueues` module (component `eventqueues`,
section `[inbound]`). Inbound events are ingested into the module, which
persists them on the host's storage, rejects redeliveries, filters and paces
them, and calls the host back (`Deliver`) when each is due. The module itself
is reached only through `tinyeventqueues-bus` (`crate::modules::event_queues`).

Two kinds of event take this path:

- **Triggers** (Composio, agent webhooks): one event per turn, triaged on
  delivery ([`deliver.rs`](./deliver.rs)), the decision applied in its own
  task.
- **External sources** (`[inbound.sources.<name>]`, e.g. `x:mentions` from an
  X bot): pushed over `inbound.offer`, debounced per key into batches, each
  batch one turn of the source's pinned agent, no triage
  ([`pinned.rs`](./pinned.rs)). The agent's final text is stored
  ([`results.rs`](./results.rs), collection `inbound.results`) and read over
  `inbound.results`, then acknowledged with `inbound.result_ack`.

| file | role |
| --- | --- |
| [`settings.rs`](./settings.rs) | `[inbound]` resolved: queues, pipeline (with one policy per external source first), backend, whether routing is on |
| [`events.rs`](./events.rs) | a trigger envelope or an external event as an `InboundEvent`, and back |
| [`route.rs`](./route.rs) | the hand-off; the direct path when the queues do not take an event |
| [`deliver.rs`](./deliver.rs) | `Deliver` and `ClassifySalience`: triage or the pinned agent, ack semantics, retries, dead letters |
| [`pinned.rs`](./pinned.rs) | rendering a batch's prompt, running the pinned agent under `external_inbound_origin`, resuming unfinished turns at boot |
| [`results.rs`](./results.rs) | pinned turns' results: `running`, `completed`, `failed`; listing, ack, TTL pruning |
| [`host.rs`](./host.rs) | the served `ai.tinyhumans.tinyeventqueues.Host` callbacks |
| [`ops.rs`](./ops.rs), [`schemas.rs`](./schemas.rs) | `inbound.stats`, `dead`, `redrive`, `offer`, `results`, `result_ack` |

## External sources

```text
pusher ── inbound.offer ──▶ queues (dedupe on source:event_id, debounce per key,
                                    max_batch cap, interactive lane, instant)
                                 │ Deliver(turn)
                                 ▼
                      per-event injection screen (blocked → result.blocked)
                                 │
                      pinned agent turn (no triage, fresh thread,
                      TrustedAutomation{Workflow, require_approval})
                                 │ text
                                 ▼
             inbound.results ◀── store ──▶ InboundTurnCompleted (wake only)
```

- Each event's text is screened with the prompt-injection guard
  `inference.agent_chat` uses before the prompt is rendered. A blocked event
  is left out and listed on the result as `blocked: [{event_id, reason}]`
  (verdict and rule codes, no text). Every event blocked: the turn completes
  with empty `text`, no model call, and `InboundTurnCompleted` is still
  published.
- The events carry no text; the SSE stream only wakes a consumer. Read the
  text over `inbound.results`.
- A delivery waits for the agent within its budget (28 s, under the module's
  30 s callback deadline). Finished: `completed`, accepted. Failed: retried
  with the lane's back-off, stored `failed` and `InboundTurnFailed` published
  on the last attempt. Still running: accepted, and the run stores its own
  result (`completed` or `failed`) when it ends.
- A finished turn is never run again on redelivery. A turn left `running` by
  a stopped process is run again at the next start from the message it
  stored.
- Results older than the source's `result_ttl_days` are pruned when results
  are read.

The per-source debounce relies on `sources[].coalesce` in the module's
pipeline, which a `tinyeventqueues` release before it rejects; configure an
external source only with a module that has it.

## Tests

Beside each module (`*_tests.rs`); [`linked_fixture.rs`](./linked_fixture.rs)
links the real module into the test binary on a private bus with scripted
triage and a scripted pinned agent. `ops_tests.rs` drives offers end to end
through it (three offers in the window: one turn, one result, one
`InboundTurnCompleted`; a burst over `max_batch`).
