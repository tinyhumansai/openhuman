# Embedded budgets and fanout

`Completer::budget` and `Turn::budget` accept `budget::ModelBudget`. Its ledger
is shared by clones, tool-loop calls, retries, fallback routes, compaction and
synchronous subagents. `Budget::child` adds a local ceiling while every call
also debits every ancestor. Use one fresh root ledger per review/run.

```rust,no_run
use openhuman_embed::budget::{Budget, CallBudget, ModelBudget, SpendLimits};
let run = Budget::new(SpendLimits {
    tokens: Some(200_000),
    cost_micros: Some(2_000_000), // $2
});
let turn = ModelBudget {
    ledger: run.child(SpendLimits {
        tokens: Some(40_000),
        cost_micros: Some(400_000),
    }),
    call: CallBudget {
        input_tokens: 10_000,
        output_tokens: 1_000,
        cost_micros: 100_000,
    },
};
```

Each physical provider call reserves input plus its capped output tokens and
the cost upper bound under one shared lock. Completed spend plus every live
reservation must fit the turn and ancestor ceilings. Usage reconciliation is
atomic too. Once refused, no provider request is sent; the facade returns
`CoreError::BudgetExceeded` with completed spend, outstanding reservations,
requested reservation and refusing limits.

Choose conservative per-call bounds for **all allowed routes**, including
fallback prices, reasoning and cached-input billing. This is pre-call admission,
not a provider-side monetary cap: an incorrect bound or a provider violating its
output cap can exceed the reservation. Actual excess is recorded and refuses
further calls. Missing cost, failed requests and cancellation retain the cost
reservation; unknown token usage retains the token reservation. This prevents
lost responses from quietly restoring spend that may already have been billed.

Budgeted calls currently accept text input. Serialized message and tool-schema
bytes bound input tokens conservatively; the host's input bound must also leave
room for provider framing. Non-text inputs and pass-through output cap overrides
are refused before dispatch. Normal calls without a budget retain their existing
modality support.

## Ordered bounded fanout

`Runtime::fanout` and `fanout::fanout` accept `Vec<Branch>`, a `NonZeroUsize`
concurrency bound and a shared `ModelBudget`. The free function suits stateless
completers without a runtime. `LeafCall` carries a completion or an owned turn;
`Branch::limits` adds a branch ceiling to the shared ledger.

Every result occupies its input slot regardless of finish order. A root error
skips only its own children; errors in children are individually returned and
leave later children running. At most the declared number of branches execute
at once. A branch runs its root then its children sequentially. Dropping fanout
drops the in-flight futures; it leaves no spawned background calls.

`Branch::children` accepts only `LeafCall`, which has no children method. Model
subagent depth is also narrowed at turn admission: root calls permit at most one
level and declared leaf calls permit none. The explicit host carrier inherits
the ceiling; TinyAgents and the OpenHuman spawn boundary check it before calls.
An unset agent session gets a fresh session ID. Hosts should supply distinct
session identities when setting them explicitly.

Validation lives in `crates/openhuman-embed/tests/budget_fanout.rs`,
`structured_turns.rs`, the explicit run-carrier tests, and TinyInference's
`model/budget_tests.rs`. All provider fixtures run locally with wiremock or
scripted models.

Hosts with borrowed tree readers or custom mock model futures can use
`fanout_futures(Vec<BranchFuture<'a, T, E>>, concurrency)` instead. Its boxed
futures may borrow local inputs and retain their own error type. This scheduler
is also used by the typed primitive. Configure each model future's budget
explicitly; an opaque future may be entirely offline and is not assumed to
perform inference.
