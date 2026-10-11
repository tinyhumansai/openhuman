# Completion routing

`Completer::complete` remains a single explicit endpoint/model choice. Hosts
that want fallback use `routing::CompletionLadder`:

```rust,no_run
use openhuman_embed::{ChatMessage, Completer, CompletionRequest, Route};
use openhuman_embed::routing::{CompletionLadder, CompletionRung, TruncationRetry};

# async fn run(key: &str) -> Result<(), Box<dyn std::error::Error>> {
let completer = Completer::new(Route::openrouter(key));
let ladder = CompletionLadder::new(CompletionRung::new(completer.clone(), "openai/gpt-4.1-mini"))
    .fallback(CompletionRung::new(completer.clone(), "moonshotai/kimi-k2.5"))
    .fallback(CompletionRung::new(completer, "minimax/minimax-m3").unpinned())
    .truncation_retry(TruncationRetry::new(2, 4096));
let outcome = ladder.complete(
    CompletionRequest::new("overridden-by-rung", vec![ChatMessage::user("Describe the attached image.")
        .with_image("https://example.org/image.png")])
        .max_tokens(1024)
        .provider_options(serde_json::json!({"provider": {"only": ["preferred"]}, "usage": {"include": true}})),
).await?;
assert_eq!(outcome.attempts.last().unwrap().answered_model, outcome.response.answered_model);
# Ok(())
# }
```

Each rung starts with the original conversation. By default it inherits the
request cap and provider options. `CompletionRung::provider_options(value)`
replaces the options for that rung, and `max_tokens(Some(cap))` overrides its
initial cap. `max_tokens(None)` explicitly removes an inherited cap and disables
truncation growth; omitting the builder inherits the request cap. Options are
replaced before `unpinned()` removes the provider routing object. Rung debug
output omits option values. Each fallback retries from its own initial cap.

A case-insensitive `length` or `max_tokens` finish
reason retries the same rung at doubled caps, bounded by both the retry count
and absolute ceiling; 1024 tokens with the example policy tries 1024, 2048,
4096. A missing cap never creates an implicit token budget. At the ceiling,
routing advances. RPC provider/transport failures also advance; local route
or security errors stop immediately. Error strings are never classified as
HTTP statuses. A route's own provider compatibility behavior remains owned
by TinyInference.

`CompletionRequest::timeout_ms(120_000)` bounds each physical provider HTTP
request to 120 seconds. Each ladder attempt and structured repair dispatch uses
that bound; it is not a total ladder deadline. With the field unset, transport
defaults remain unchanged. A physical timeout is an RPC transport failure and
can advance to the next rung. Any timed-out attempt has unknown cost, so the
ladder's aggregate cost remains `None`; budgeted calls retain the conservative
reservation when no authoritative usage arrived, which can block later calls.

`Completer::timeout(duration)` instead bounds the entire logical call, including
structured repairs. Its typed `DeadlineExceeded` is terminal, and cancellation
is also terminal: neither advances to a fallback. Hosts can use both timeouts
when they need a physical request bound and a separate logical deadline.

An `unpinned()` rung must be last. It removes the gateway's `provider` object,
while keeping model, reasoning, usage options and images. This lets the final
gateway route choose its serving provider without changing the requested
model. Adding a rung after an unpinned rung fails before any dispatch.

`response.usage` describes the winning call. `attempts` records every call,
including truncated responses. Buyer `usage.buyer_cost_micro` charges take
precedence over relayed `usage.cost`, then normalized `charged_amount`. The
selected amount must be finite and nonnegative; invalid selected charges stay
unknown. `total_usage` sums reported tokens and costs;
its cost is unknown (`None`) when any attempt's cost was missing or invalid. The
ladder error retains this accounting and the last typed error. Completer
observers run for each call, so hosts can meter failures independently.

`Route::{openrouter, moonshot, minimax}` and matching `Provider` constructors
use the international API roots. Custom deployments, regional endpoints and
loopback fixtures use `openai_compatible`.

# Agent exploration

`Turn::provider_options` applies gateway routing/reasoning options to every
call in a runtime-owned agent turn. `Turn::require_tool_call(true)` requires a
successful tool execution before accepting a terminal answer. Until then,
the host sends `tool_choice: "required"` and withholds the final response
schema so the model can choose a tool. A gateway that ignores the hint and
returns terminal JSON is refused. A rejected or failed tool does not satisfy
the requirement. After successful execution the final schema applies again.
The requirement is per turn, so prior session reads do not satisfy it.

This is an explicit host contract: a prompt saying “read first” alone cannot
guarantee exploration. The fixtures prove native tool metadata is advertised
for the tested OpenRouter model IDs and that premature replies fail; they do
not claim live provider behavioral parity or independently prove a remote
model's implementation.
