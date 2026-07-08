# Phase 4 — Factory, registry lifecycle, heartbeat fan-out, RPC

Goal: the "make subconscious" surface — instantiate any set of worlds, drive
them from the heartbeat, expose per-instance status/trigger over JSON-RPC.

## 4.1 `factory.rs`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubconsciousKind { Memory, TinyPlace }

impl SubconsciousKind {
    pub fn id(self) -> &'static str;               // "memory" | "tinyplace"
    pub fn parse(s: &str) -> Option<Self>;
    /// Which kinds should run for this config (bootstrap set).
    pub fn enabled_kinds(config: &Config) -> Vec<Self> {
        // Memory   ⇐ heartbeat.enabled && mode != Off        (today's gate)
        // TinyPlace ⇐ orchestration.enabled                   (today's gate)
    }
}

pub fn make_subconscious(kind: SubconsciousKind, config: &Config) -> SubconsciousInstance;
```

`make_subconscious` is the *only* place profiles are constructed — tests and
the trigger RPC go through it too, so a new kind is: profile file + one match
arm + one `enabled_kinds` line.

## 4.2 `registry.rs` (grown from `global.rs`)

Replace the single `OnceLock<Arc<Mutex<Option<SubconsciousEngine>>>>` with a
keyed registry:

```rust
static REGISTRY: OnceLock<Mutex<HashMap<SubconsciousKind, Arc<SubconsciousInstance>>>>;
```

- `get_or_init_instance(kind)` — lazy per-kind construction (same
  load-config-then-insert dance as today's `get_or_init_engine`).
- `bootstrap_after_login()` — unchanged guard (`BOOTSTRAPPED` swap), then
  initializes every `enabled_kinds(config)` member, spawns the heartbeat, and
  (unchanged) the opt-in trigger orchestrator.
- `stop_heartbeat_loop()` / `reset_engine_for_user_switch()` — abort the
  heartbeat, shut down the trigger orchestrator, then clear the whole map so
  a user switch rebuilds every instance against the new workspace.
- Keep `get_or_init_engine()` as a deprecated alias for
  `get_or_init_instance(Memory)` until the RPC handlers are ported (below),
  then delete it.

Instances stay `Arc` (not `Mutex<Option<..>>`): the instance's own
`tick_lock`/`state` mutexes already serialize what needs serializing, and the
status path must remain lock-free per invariant 5.

## 4.3 Heartbeat fan-out

`heartbeat/engine.rs` currently calls the single engine's `tick()` on its
interval. Change: on each heartbeat interval, iterate the registry and tick
every instance whose `cadence` has elapsed since its `last_tick_at`
(per-instance keys from phase 1 make this a pure read). Instances tick
**concurrently** (`tokio::spawn` per instance, joined with the existing
cancel/abort semantics) — a slow memory tick must not delay a tinyplace
review. The heartbeat's event-planner duties (meetings/reminders) are
untouched.

## 4.4 RPC surface (`schemas.rs`)

Backward-compatible extension of the `subconscious` namespace:

- `subconscious.status` → today's top-level fields stay, populated from the
  **memory** instance (existing UI keeps working), plus
  `instances: [SubconsciousStatus]` with one row per registered kind
  (each row gains `instance: "memory" | "tinyplace"`).
- `subconscious.trigger` → optional `kind` param (`"memory"` default —
  today's behavior; `"tinyplace"`; `"all"`). Still fire-and-forget
  (spawned, returns immediately).
- Status reads stay SQLite-only: per-instance `last_tick_at` comes from the
  namespaced KV; in-memory counters (failures, halt reason) come from the
  instance's `status()` which takes only the small `state` mutex, never
  `tick_lock` — same as today.

Frontend consumption of these fields is phase 7 (instance cards in the
Subconscious tab, steering header in the TinyPlace Orchestration tab). Not a
blocker for the Rust work — the additions are backward-compatible.

## 4.5 `about_app`

Update `src/openhuman/about_app/` copy: the subconscious is now described as
per-world instances (memory awareness + tiny.place orchestration steering),
per the repo rule that user-facing feature changes update about_app.
