# iMessage live-tick harness

**Worktree:** `.worktrees/imessage-live-harness`
**Branch:** `feature/imessage-live-harness`
**Goal:** make scanner's tick body testable against real chat.db without Tauri AppHandle / CEF / UI — becomes template for 5 more Apple-native sources.

## Problem

Scanner tick body (`app/src-tauri/src/imessage_scanner/mod.rs:87-210`) is coupled to:
- `AppHandle` (cursor path via `tauri::Manager::path`)
- Hardcoded HTTP to `http://127.0.0.1:7788/rpc` for gate + ingest
- Fire-and-forget loop (not a pure function)

Can't run one tick in a test without spinning full Tauri app.

## Shape

Extract a pure async fn:

```rust
pub struct TickInput {
    pub db_path: PathBuf,
    pub last_rowid: i64,
    pub account_id: String,
}

pub struct TickOutcome {
    pub new_rowid: i64,
    pub groups_attempted: usize,
    pub groups_ingested: usize,
    pub skipped_unconnected: bool,
}

#[async_trait]
pub trait TickDeps {
    async fn fetch_gate(&self) -> anyhow::Result<Option<Vec<String>>>;
    async fn ingest_group(&self, account_id: &str, key: &str, transcript: String) -> anyhow::Result<()>;
}

pub async fn run_single_tick<D: TickDeps>(input: TickInput, deps: &D) -> TickOutcome;
```

Prod path wraps: `HttpDeps { base_url }` implements `TickDeps` with existing JSON-RPC calls.

`run_scanner` loop becomes: cursor load (AppHandle) + `loop { run_single_tick(...).await; sleep }`.

## Test layers

1. **Unit** (default `cargo test`): `FakeDeps` records ingest calls, returns configurable gate. Real chat.db. Assert: N groups ingested matching chat_allowed filter, cursor advanced, fail-group path keeps cursor.
2. **Ignored live** (`cargo test -- --ignored`): spawn real `openhuman` sidecar on ephemeral port + temp `OPENHUMAN_WORKSPACE`. Hit `config_set` to enable iMessage. Run one tick. Query temp memory.db for `namespace LIKE 'imessage%'`. Assert ≥1 row.

## Steps (TDD order)

- [ ] Step 1: Write failing unit test `run_single_tick_ingests_groups_from_real_chatdb` using fake deps. Red.
- [ ] Step 2: Extract `TickDeps` trait + `TickInput`/`TickOutcome` + `run_single_tick` (stub). Compile.
- [ ] Step 3: Move tick body 129-209 into `run_single_tick`. Green.
- [ ] Step 4: Write `HttpDeps` wrapping existing gate+ingest fns. Use in `run_scanner`.
- [ ] Step 5: Existing 9 unit tests + 2 ignored still green.
- [ ] Step 6: Write ignored live-sidecar test. Needs sidecar binary staged; use `cargo build --bin openhuman` at root first.
- [ ] Step 7: Update `docs/superpowers/runbooks/imessage-verification.md` with harness commands.
- [ ] Step 8: Run `cargo check` + `cargo fmt` in app/src-tauri.

## Non-goals

- UI changes
- Cursor file format changes
- Multi-account (keep account_id string as-is)
- Generalizing trait for 5 other sources *yet* — prove pattern on iMessage first, extract after.

## Layer 3 (click-through A)

After harness + tests green: `pnpm tauri dev`, connect iMessage in Settings, verify live tick in UI path. Separate from code.
