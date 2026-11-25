# Controller parser contract migration

## Objective and acceptance

Complete the broader version of senamakel/openhuman#228 after #466: every controller in the built-in core registry and every hosted extension must have an independent method/required-input contract and explicit valid example(s) accepted by the same pure decoder/preprocessor used by its runtime handler. The sweep must not call async handlers or cross a configuration, permission, module, network, filesystem, credential, or business-operation boundary. Core and hosted coverage must be exercised with relevant feature sets enabled and disabled.

Preserve the issue's size acceptance: shrink the counted `*schemas_tests.rs` corpus by at least 50% from the agreed 8,401-line baseline (maximum 4,200 lines). Count every replacement test, fixture, helper, macro definition/invocation, and extracted test-only file in the same accounting footprint; do not credit moved code or hide tests outside the glob. Maintain a second conservative accounting measure including all net crate-source lines added outside that glob. Keep behavior, security, defaults, migration, wire, error, and domain-output assertions unless a specific assertion is proven redundant against an independent oracle.

The existing receipts establish that universal parser coverage is feasible as a staged test contract, but do not establish the 50% reduction. The accepted base has a conservative footprint of about 7,920 lines, leaving a 3,720-line gap to 4,200; current audited candidate lists do not justify that deletion budget. Coverage work must therefore proceed alongside an assertion-by-assertion compression ledger. At each wave boundary, stop expansion and report the remaining gap if the only further reductions would remove independent behavior/security oracles. Do not claim the size criterion is met from moved helpers, count reductions, compressed formatting, or deleted meaningful behavior. Root review is required before accepting any proposed tradeoff.

## Architecture

Use a production registration object that binds runtime invocation and parser inspection in one constructor. A fixture must no longer carry a separate decoder function pointer or handler pointer that can drift from the executable registration. Required names and examples remain independent test data; the decoder comes from the exact registration object used by dispatch.

The proposed production types live in `crates/openhuman-core/src/core/all.rs` (or a small sibling module):

```rust
type Params = serde_json::Map<String, serde_json::Value>;
type ParseParams<P> = fn(Params) -> Result<P, String>;
type RunController<P> = fn(DispatchCtx, P) -> ControllerFuture;
type ControllerTask<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'static>>;
type Gate<G> = fn(DispatchCtx) -> ControllerTask<G>;

pub struct BoundRegisteredController {
    pub schema: ControllerSchema,
    handler: BoundHandler,
}

impl BoundRegisteredController {
    pub fn typed<P: Send + 'static>(
        schema: ControllerSchema,
        parse: ParseParams<P>,
        run: RunController<P>,
    ) -> Self;

    pub fn after_gate<G: Send + 'static, P: Send + 'static>(
        schema: ControllerSchema,
        gate: Gate<G>,
        parse: ParseParams<P>,
        run: fn(DispatchCtx, G, P) -> ControllerFuture,
    ) -> Self;

    pub fn decode_params(&self, params: Params) -> Result<(), String>;
    pub fn invoke(&self, ctx: DispatchCtx, params: Params) -> ControllerFuture;
}
```

`BoundHandler` is an erased private pair of runtime invocation and decoder inspection, constructed only by these constructors. `typed` stores one `parse` pointer and clones that same pointer into an async invocation wrapper; its wrapper runs `parse(params)` when polled, then calls `run(ctx, typed)`. `after_gate` runs the async gate first, then calls the same stored parser, then calls `run(ctx, gate_value, typed)`. No public constructor accepts an already-erased runtime handler plus an unrelated decoder. The contract runner reads `decode_params` from the controller in the registry, so a fixture cannot supply or substitute its own callback. If a distinct required ordering is found, add a named constructor that encodes that sequence rather than a free-form parallel callback.

The signatures above are a design target, not a claim that all current handlers fit `typed` directly. For example, `threads::schemas::handlers::handle_upsert` parses first, then checks user-surface thread and parent ids, then invokes the operation. Its bound `run` function must retain those checks between decode and operation. `integrations::composio::schemas::handlers_tools::handle_execute` loads config first, parses `tool`/`arguments`/`connection_id` second, then calls the operation; use `after_gate` with config as `G`, and preserve the existing readers and error order. `modules::schemas::handle_computer_status` reads `load` before loading config and checking the computer component; use `typed` with a parser returning the bool, then keep config load and component check in `run`. Thus the parser observation is side-effect-free, while runtime ordering stays exactly where it is today.

Registration compatibility is explicit. Keep today's public `RegisteredController { schema, handler: fn(...) }` name, fields, and struct-literal construction unchanged. Introduce a distinct constructor-only `BoundRegisteredController`; it exposes a read-only schema and private bound handler. Keep today's `ControllerExtension { group, controllers: Vec<RegisteredController>, namespaces }` and `register_controller_extension` unchanged. Add `BoundControllerExtension { group, controllers: Vec<BoundRegisteredController>, namespaces }` and `register_bound_controller_extension`. Internally store `GroupedController { group, entry: ControllerEntry }`, where `ControllerEntry::{Legacy(RegisteredController), Bound(BoundRegisteredController)}` dispatches each variant through its own invoke path. Add crate-internal `ControllerEntry::legacy` and `ControllerEntry::bound` conversions so a domain's `all_registered_controllers` can migrate one registration at a time without changing public legacy constructors. Migrate in-repository providers and snapshot consumers namespace by namespace; do not silently label the old live projection complete. During rollout, contract tests report legacy rows explicitly. In the final state the raw built-in/internal and each owner extension snapshot reject `Legacy` entries. This preserves existing external struct literals while exposing migration debt and a constructor-only path for complete coverage.

The concrete registration diff for the first typed vertical slice is:

```rust
// before
RegisteredController { schema: schemas("upsert"), handler: handle_upsert }

// after
BoundRegisteredController::typed(schemas("upsert"), parse_upsert, run_upsert)
```

The manual gate-before-parse registration is:

```rust
BoundRegisteredController::after_gate(
    schemas("execute"),
    |ctx| Box::pin(async move { ctx.load_config().await }),
    parse_execute,
    |ctx, config, parsed| Box::pin(async move {
        to_json(ops::composio_execute(
            &config, &parsed.tool, parsed.arguments, parsed.connection_id.as_deref()
        ).await?)
    }),
)
```

The final closure only receives already parsed input. The constructor owns the stored parser pointer and creates both its `decode_params` view and its invocation future from that pointer. Existing manual semantic checks remain in `run_*` at their present point after parsing; handlers with more than one gate/parser ordering need a named constructor representing their actual ordered stages before migration.

The test-only fixture for each registration contains only an independent method literal, independent required names, and explicit valid JSON examples. The verifier joins fixtures to registered controllers by method, rejects duplicates/empty inventories/missing or extra rows, checks handler/schema method identity, nonempty descriptions, and exact required declarations, then verifies each example with both `validate_params` and `controller.decode_params`. Required-field deletion/optional mutants alter cloned schemas and must fail against independent literal expectations. Existing error/default/value tests remain wherever they express behavior beyond this parser acceptance boundary. Business operation success, state-dependent validation, and environment-dependent checks stay in focused owner tests.

The core owns built-in and internal controllers. `openhuman-tinyhumans` owns two additional RPC extension inventories: `hosted::extension()` in `DomainGroup::Hosted`, and the separate `voice::schemas::hosted_voice_controllers()` extension in `DomainGroup::Voice` (`voice.agent_signed_url`, `voice.cloud_transcribe`). `openhuman-rpc` owns the `http_host::extension()` inventory in `DomainGroup::Platform`. Each owner has a local contract test over the exact extension object it constructs; core must not depend upward or mutate a process-global extension registry in parallel tests. Feature-gated rows are guarded by the same feature and each supported relevant build configuration runs its contract suite. No vendor module implementation is imported to construct a host test.

## Migration rules

For every old test considered for removal, record its asserted behavior, its replacement oracle (or why it is a true duplicate), and old/new line counts. Preserve independent exact identities, required/optional distinctions, output names/shapes, secret redaction, error classes/messages relied on by callers, defaults, aliases, migrations, and security policy. Generic registry consistency does not replace these owner-level contracts. Do not derive fixtures, required names, examples, or expected method inventory from the live schema. Do not run side-effecting handlers as test probes. Do not move tests outside the counted footprint to claim savings.

Required-field mutation checks must run over every contract row and every independently declared required name: remove the field from a cloned schema and separately mark it optional; the runner must reject both. The ordinary positive contract must also fail against a deliberate required-schema regression. Demonstrate red with a nonzero selected test count, restore the exact source, then rerun green. A failing callback or error caused by config/gate setup is not acceptable evidence of parser rejection.

## Bite-sized TDD sequence

Each step is a separately reviewable change on `epic/8089-test-ownership`; keep the branch buildable. For each behavior change, add the test first, run the focused test and record its red result, implement the smallest change, then rerun it green. Keep formatting and focused checks for touched files in that step.

### 1. Establish the bound registration primitive with one typed controller

Files: `crates/openhuman-core/src/core/all.rs`, `all_invoke.rs`, a sibling bound-registration test file (declared with the repository's sibling `*_tests.rs` rule), `schema_test_support.rs`, its sibling `schema_test_support_tests.rs`, and `all_schema_contract_tests.rs`; first owner migration: `crates/openhuman-core/src/threads/schemas.rs`, `threads/schemas/handlers.rs`, and `threads/schemas_tests.rs`.

First add a typed constructor test with a test parser and side-effect counter: `invoke` must call the parser and typed runner in order; `decode_params` must call that exact parser and must not call the gate or runner. Test parser errors propagate unchanged. Then implement `BoundHandler` and internal `ControllerEntry` dispatch; preserve the outer `try_invoke_registered_rpc` domain/surface gate before any parser runs. Do not change or rename legacy `RegisteredController` or `ControllerExtension` fields/literals. Add `BoundRegisteredController`, `BoundControllerExtension`, and new APIs alongside them. Add a generic `push_entries` path for new registrations; retain `push(Vec<RegisteredController>)` as the legacy-to-entry adapter. Migrated domain providers return `Vec<ControllerEntry>`, while unconverted providers keep their old return type until their namespace migrates. Keep both paths building during rollout. The final raw built-in/internal and all three owner extension inventories must contain no legacy entries, and their completeness tests must fail if any remain.

The first production vertical slice is `threads.upsert`. In `threads/schemas/handlers.rs`, extract `parse_upsert(params) -> Result<UpsertConversationThreadRequest, String>` from the existing generic `parse::<...>` use, and `run_upsert(ctx, request)` that retains thread-id checks before `ops::thread_upsert`. In `threads/schemas/registry.rs`, replace only the `upsert` literal with `ControllerEntry::Bound(BoundRegisteredController::typed(schema, parse_upsert, run_upsert))` and convert sibling literals using `ControllerEntry::legacy(RegisteredController { ... })`. The provider returns `Vec<ControllerEntry>` for this staged namespace; its other entries remain explicitly legacy. `push_entries` preserves those variants and runtime dispatch continues through `GroupedController`; this mixed vector is explicit, detectable debt, not complete coverage. Independently fixture `threads.upsert`, its exact required keys, and a valid request. The registry contract test calls `controller.decode_params`, not a fixture callback. Add runner regressions for missing/extra/duplicate rows, empty inventory, empty examples, wrong fixture identity, schema rejection, parser rejection, required key removal, and required-to-optional mutation. Keep current memory/wallet fixed assertions until replaced by complete rows.

TDD proof: first demonstrate the fixture contract rejects a deliberately removed/optional `threads.upsert` required schema field (nonzero selected test count); restore it and show green. Separately, temporarily alter the production `parse_upsert` to reject a valid example and prove the ordinary registry contract turns red; restore and rerun green. Do not call `run_upsert` in either contract test.

### 2. Add an order-preserving gate constructor and bind a manual parser

Files: `crates/openhuman-core/src/core/all.rs`, `all_invoke.rs`, bound-registration tests, `integrations/composio/schemas/handlers_tools.rs`, its registration file and sibling tests, and `core/all_schema_contract_tests.rs`.

Add `after_gate` with a test proving call order is gate → parser → run and that gate failure prevents parser execution. Extract Composio's exact existing readers into `parse_execute`; `after_gate` obtains `ctx.load_config().await?`, invokes `parse_execute`, and passes config plus parsed values to the operation runner. Retain missing/blank/wrong-type `tool`, absent/null/scalar/object `arguments`, and `connection_id` behavior/error tests. The contract sweep uses a side-effect-free parser accessor on the production registration and never loads config or calls the operation. Use this same stage model for each manual parser whose current gate precedes parsing; use `typed` for `modules.computer_status` because its `load` preprocessing currently precedes config and component checks.

Only after the binding seam is proven, migrate the existing voice, MCP registry, and computer-status contracts from fixture callbacks to registration-owned decoders. Voice needs every registered method, not a representative filtered set. Preserve the old independent voice behavior/default/provider/error/security oracles. Keep `computer_status_load` before config/component checks. Add enabled/disabled feature coverage, explicit inventory set equality, and required-field mutants. Do not compress domain tests until replacements are active and line accounting is updated.

### 3. Convert all built-in and internal registrations to bound entries

Files: core registration owners under `crates/openhuman-core/src/**/schemas*.rs` and registration lists, `core/all.rs`, `all_invoke.rs`, test support, and each owner's sibling tests.

For each domain, migrate a small complete namespace (roughly 5–20 controllers) from bare registrations to `BoundRegisteredController::typed`, `after_gate`, or another explicitly order-preserving constructor. A handler with no input still binds an explicit empty-map parser; a handler that ignores params must state that behavior in its decoder. Extract named pure parsers only for actual runtime reads, not merely to make tests pass. Keep handler-specific checks in the typed runner at the original point relative to config, identity, permissions, feature gates, and operations. Each namespace change includes its complete independently authored method/required/example rows and replaces its owner schema-only assertions only after those rows and behavior oracles pass.

Use `builtin_controller_snapshot()` in `core/all_schema_contract_tests.rs` as the test corpus: it chains raw `registry()` plus `internal_registry()` directly, before ambient `DomainSet`, `visible`, or surface filtering. Keep public `all_registered_controllers()` behavior unchanged; it is a live filtered product API, not the universal test source. Do not let fixture rows determine the selected registry slice. Feature-gated groups use matching compile-time guards and enabled/disabled Cargo builds. Apply required-field mutants to every row after each namespace wave. Migrate in ownership batches for review: threads/sessions/profiles; agent runs/workflows; security; config/tools/inference; artifacts/skills/integrations/MCP; cron/channels/core voice; desktop/platform/storage/web3/flows/memory/sandbox/health. These are candidate ordering groups, not large one-PR tasks: split each so the PR has one vertical namespace or a few tightly related ones and remains buildable. Keep the per-domain old/new line and assertion ledger.

### 4. Migrate each extension owner without a legacy coverage hole

Files: `crates/openhuman-core/src/core/all.rs` extension types and registration; `crates/openhuman-embed/src/runtime/seams.rs`; `crates/openhuman-tinyhumans/src/runtime.rs`, `install.rs`, `hosted/**`, `voice/schemas.rs`; `crates/openhuman-rpc/src/http_host/**` and host setup/tests.

Preserve the current `RegisteredController`, `ControllerExtension`, and `register_controller_extension` APIs exactly. Add `BoundControllerExtension` with `Vec<BoundRegisteredController>` and a separate registration function; core stores both in `ControllerEntry` and never converts a bound entry back to a legacy handler pointer. TinyHumans' `hosted::extension()` migrates its proxy controllers through this new path. Separately migrate `tinyhumans/src/voice/schemas.rs`: its `voice.agent_signed_url` and `voice.cloud_transcribe` entries are a Voice-group extension, distinct from the Hosted-group proxy extension, and need their own fixed contract table and owner tests. Each extension constructor returns the actual bound extension object tested locally before install.

The extension path already crosses a fixed facade chain. Add only the necessary type/function re-exports to `openhuman_embed::__host` (`crates/openhuman-embed/src/lib.rs`), relying on `openhuman-tinyhumans/src/lib.rs`'s existing `pub use openhuman_embed::__host`; RPC already reaches that same list as `openhuman_tinyhumans::__host as core_host`. Do not add a core dependency upward or bypass the facade. Preserve the existing embed builder call `.controller_extension(ControllerExtension)` and add `.bound_controller_extension(BoundControllerExtension)` in `crates/openhuman-embed/src/runtime/seams.rs`; store legacy and bound options in separate vectors and install each through its matching registration function. In the TinyHumans generated `RuntimeBuilder` (`src/runtime.rs`), add a parallel bound setter and forward it to embed. Extend internal `Wiring` preparation and `install` so the existing `controllers: Option<ControllerExtension>` legacy path remains intact while a `bound_controllers: Option<BoundControllerExtension>` path carries the parser binding without conversion. In `RuntimeBuilder::controller_extension`, keep its existing setter and generated forwarding from TinyHumans; add a separate bound setter in TinyHumans runtime. Keep public `InstallOptions` fields and meanings unchanged; add a bound sibling to the private `Wiring.controllers` payload and update `prepare`/`install` plumbing to retain both the legacy and bound extension values until their matching registration calls. Migrate hosted proxies and TinyHumans voice registration separately; preserve voice registration's current boot timing and duplicate/idempotency behavior.

RPC owns a third extension: `http_host::extension()` under `DomainGroup::Platform`, not Hosted or Voice. Convert `crates/openhuman-rpc/src/http_host/schemas.rs` registrations to bound constructors and `http_host::extension()` plus `register_controllers`/`ensure_registered` to the bound extension path. Update `host.rs` builder setup to call `.bound_controller_extension(...)`, and preserve the legacy builder method for callers. Keep contract tests in `crates/openhuman-rpc/src/http_host/schemas_tests.rs`; add explicit fixed required names/examples and mutants there. Add facade forwarding tests/checks through embed → tinyhumans → rpc and verify no product Cargo feature forwarding is changed unless an existing gate actually needs forwarding. Run core, tinyhumans (including `voice` and `hosted` configurations), and rpc `server`/relevant feature tests in their owning crates.

The core's cfg(test) runner is not shared into dependent crate tests, so tinyhumans and rpc use a small local contract loop over their exact bound extension object. Each calls the `decode_params` method bound into its `BoundRegisteredController`; neither passes a parallel callback. Gate and hosted-auth order remains inside its bound pipeline and is never run by the test sweep. No network client, auth exchange, extension global, or hosted operation is invoked. Test full extension parity, uniqueness, independent identity/required fields, descriptions, required mutants, and parser acceptance under each owner's supported feature configuration.

### 5. Remove only proven duplicate schema-test scaffolding

Files: remaining core `*_schemas_tests.rs`, config schema tests, and any test-support files originally in scope.

For each candidate, replace repeated generic name/count/parity/field traversal with compact readable fixed tables using shared helpers. Keep literal expected method names and independent values visible. Retain all behavior/default/security/migration/wire/error/output assertions. Delete compile-only stubs only after confirming they assert no behavior and are not required for module linkage. Remove serde-default tests only when the default is asserted independently elsewhere or has no product/security/migration meaning. Do not mechanically minify or macro-hide fixtures. Review size against both corpus and conservative accounting after every domain batch.

### 6. Close acceptance and verification

Files: final test inventories only; update this plan or PR description with measured results, not source API documentation.

Make the global core contract test cover every raw built-in and internal registered method exactly once for the enabled feature set; fail if any `ControllerEntry::Legacy` remains in that snapshot. The disabled-feature build proves no unavailable contracts leak in. Independently require complete bound snapshots from (1) `openhuman_tinyhumans::hosted::extension()` (Hosted group), (2) `openhuman_tinyhumans::voice::schemas::hosted_voice_controllers()` (Voice group), and (3) `openhuman_rpc::http_host::extension()` (Platform group). Do not use the public live `all_registered_controllers()` as the coverage oracle; its ambient `DomainSet`, profile-surface filtering, and global extensions are runtime behavior intentionally preserved. Require each owner snapshot to contain no legacy entry before asserting its independent contracts. Run focused crate checks, strict all-target clippy, focused tests, and formatting for touched crates; then the issue gates: `cargo test -p openhuman --lib core::`, `cargo test -p openhuman --lib` with the repository's prescribed `RUST_MIN_STACK`, `pnpm rust:layout`, and `node scripts/check-domain-e2e-coverage.mjs`. Run tinyhumans tests with the relevant `voice`/`hosting` gates, RPC tests with `server`, and compile facade chain through embed → tinyhumans → rpc, including relevant disabled-feature checks. Long Cargo gates use `scripts/ci-cancel-aware.sh`; do not set `CARGO_TARGET_DIR`. Record unrelated existing failures separately. Calculate the final 8,401 → counted-line total and conservative total from a clean diff; acceptance requires no more than 4,200 counted lines and honest accounting of all net additions.

## File groups and ownership boundaries

- Core registry and verifier: `crates/openhuman-core/src/core/all.rs`, `all_invoke.rs`, `all_schema_contract_tests.rs`, `schema_test_support.rs`, sibling `schema_test_support_tests.rs`, plus bound-registration tests. The independent built-in oracle is raw `registry()` + `internal_registry()` as already used by `builtin_controller_snapshot`, never the public ambient-filtered view.
- Core parser and behavior owners: `crates/openhuman-core/src/<domain>/**/schemas*.rs` and sibling `*_tests.rs`; no flat business logic under `core/`.
- Hosted proxy extension: `crates/openhuman-tinyhumans/src/hosted/**`; hosted voice extension: `crates/openhuman-tinyhumans/src/voice/schemas.rs`; RPC file-host extension: `crates/openhuman-rpc/src/http_host/**`. Each test remains in its owner crate and covers its own exact extension object. The original `RegisteredController`/`ControllerExtension` APIs and builder setters remain available unchanged; new bound counterparts carry parser identity. Owner completeness sweeps require bound entries even though external callers can still use the legacy API.
- Tests remain in their owning crate. Core cannot test hosted controllers by adding a dependency on tinyhumans; tinyhumans cannot reimplement core's parser or controller semantics.
- No `vendor/` production changes are expected. If investigation finds a parser or module behavior owned by a vendor, stop and move that behavior and its tests to the owning repository before changing the OpenHuman adapter.

## Honest line and behavior accounting

Starting basis: original issue baseline 8,401; cap 4,200. Partial #466 is already included in the accepted epic base. The most recent conservative receipt counted approximately 7,920 total equivalent lines and 7,109 raw schema-test lines; receipts vary slightly by accepted-base snapshot, so before source migration recompute once from the exact merged base and preserve 8,401/4,200 as the comparison. For every change, record:

| Domain/batch | Old counted lines | New counted lines | Net counted change | Outside-glob net | Assertions removed | Independent replacement |
| --- | ---: | ---: | ---: | ---: | ---: | --- |

The target is not met until the summed counted total is at most 4,200. If an honest, reviewed assertion map shows that remaining lines are independent behavior/security contracts or necessary explicit examples and the target cannot be reached without dropping them, document the exact remaining gap and ask root to resolve the scope tradeoff before weakening any oracle. Current evidence is insufficient to promise completion of the size target; it is also insufficient to call that target impossible.
