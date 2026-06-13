# people

Contact resolution + relationship scoring (the "A5" module). Maps any of three handle kinds — iMessage handle, email, or display name — to a single stable `PersonId`, and ranks known people by a deterministic composite score (recency × frequency × reciprocity × depth) derived from observed interaction rows. Backed by its own SQLite database (people / handle aliases / interactions). Can seed itself from the macOS system Address Book (`CNContactStore`). Intentionally self-contained — per its module docstring it has no dependency on `life_capture`, `chronicle`, `nudges`, or UI; downstream integration is left to later slices.

## Responsibilities

- Canonicalize handles (lowercase/trim emails + email-style iMessage handles; whitespace-collapse display names) so the same person resolves consistently across case and spacing.
- Deterministically resolve a `Handle` to an existing `PersonId`, or mint a new `Person` skeleton on first sight (`create_if_missing`).
- Link handles together (`link`) so an email + phone + display name can be attached to one person — without ever *auto*-merging distinct identities that share only a display name or an unverified handle.
- Record interactions and aggregate them into a per-person composite score plus an explainable component breakdown.
- Rank all known people by score for `people.list`.
- Seed the store from the macOS Address Book, distinguishing "permission denied" from "no contacts".
- Persist people, handle aliases, and interactions in a dedicated SQLite DB with idempotent migrations.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/people/mod.rs` | Export-focused. Declares submodules and re-exports `all_people_controller_schemas` / `all_people_registered_controllers`. |
| `src/openhuman/people/types.rs` | Domain types: `PersonId`, `Handle` (with `canonicalize` / `as_key`), `Person`, `Interaction`, `ScoreComponents`, `AddressBookContact`. |
| `src/openhuman/people/resolver.rs` | `HandleResolver` — `resolve`, `resolve_or_create(_with_status)`, `link`, `seed_from_address_book`. The deterministic handle→PersonId logic + cross-source merge-safety contract. |
| `src/openhuman/people/scorer.rs` | Pure `score(interactions, now) -> ScoreComponents`. Recency half-life, frequency window/cap, reciprocity balance, depth cap as module constants. |
| `src/openhuman/people/store.rs` | SQLite-backed `PeopleStore` (`Arc<Mutex<Connection>>`) + process-global `OnceCell` accessor (`init` / `get`). CRUD, lookup, interaction read/write, batched interaction fetch. |
| `src/openhuman/people/address_book.rs` | `ContactsSource` trait + `SystemContactsSource` (macOS `CNContactStore` FFI via objc2) and non-mac stub; `MockContactsSource` for tests; `AddressBookError`. |
| `src/openhuman/people/rpc.rs` | Domain RPC handlers (`handle_list`, `handle_resolve`, `handle_score`, `handle_refresh_address_book`) returning `RpcOutcome<Value>`; callable directly in tests with a constructed `PeopleStore`. |
| `src/openhuman/people/schemas.rs` | Controller schemas + param-parsing adapter handlers that fetch the global store and delegate to `rpc.rs`. |
| `src/openhuman/people/migrations.rs` | Idempotent migration runner (bookkeeping table `_people_migrations`, per-migration transaction). |
| `src/openhuman/people/migrations/0001_init.sql` | Schema: `people`, `handle_aliases`, `interactions` + indexes. |
| `src/openhuman/people/tests.rs` | Cross-file integration tests for the domain. |

## Public surface

- Types: `PersonId`, `Handle` (`IMessage` / `Email` / `DisplayName`), `Person`, `Interaction`, `ScoreComponents`, `AddressBookContact`.
- `HandleResolver::{resolve, resolve_or_create, resolve_or_create_with_status, link, seed_from_address_book}`.
- `scorer::score` + tunable constants `RECENCY_HALF_LIFE_DAYS`, `FREQUENCY_WINDOW_DAYS`, `FREQUENCY_CAP`, `DEPTH_CAP_CHARS`.
- `store::{PeopleStore, init, get}` and `ConnHandle`.
- `address_book::{ContactsSource, SystemContactsSource, read, read_with, AddressBookError}`.
- `mod.rs` re-exports `all_people_controller_schemas` / `all_people_registered_controllers` for the controller registry.

## RPC / controllers

Registered via the controller registry (wired in `src/core/all.rs`). Four controllers in the `people` namespace:

| Method | Inputs | Output |
| --- | --- | --- |
| `people.list` | `limit?` (default 100, capped at 500) | `people[]` ranked by score desc — each with `person_id`, `display_name?`, `primary_email?`, `primary_phone?`, `handles[]`, `score`, `components`, `interaction_count`. |
| `people.resolve` | `kind` (`imessage`/`email`/`display_name`), `value`, `create_if_missing?` | `person_id?` (null when unknown and not creating), `created`. |
| `people.score` | `person_id` (UUID) | `person_id`, `score`, `components`, `interaction_count`. Errors if person not found. |
| `people.refresh_address_book` | — | `seeded`, `skipped`, `permission_denied`. |

`score` / composite is `recency * frequency * reciprocity * depth`, each clamped to `[0,1]`.

## Persistence

Dedicated SQLite DB managed by `PeopleStore` (open via `open_at(path)` or `open_in_memory()`; migrations run on open). Three tables (see `0001_init.sql`):

- `people` — one row per resolved person (uuid id, display name, primary email/phone, timestamps).
- `handle_aliases` — `(kind, value)` primary key → `person_id` (FK, `ON DELETE CASCADE`); `value` is the canonicalized form. This table *is* the resolver index.
- `interactions` — `(person_id, ts, is_outbound, length)` rows the scorer aggregates; indexed by `(person_id, ts DESC)` and `ts DESC`.

Migrations are tracked in `_people_migrations` and applied idempotently in a transaction. The store is exposed process-globally through a `tokio::sync::OnceCell` (`init` once at startup, `get` from controller handlers). Note: no `people::store::init` call was found outside the module — controller handlers will return "people store not initialised" until something seeds it.

## Dependencies

- `crate::core::all::{ControllerFuture, RegisteredController}` — controller registry types for RPC exposure.
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller schema definitions.
- `crate::rpc::RpcOutcome` — standard RPC result envelope (`RpcOutcome<T>`).
- External crates: `rusqlite` (storage), `tokio` (async + `spawn_blocking` for sync SQL, `OnceCell`, `Mutex`), `chrono` (timestamps/scoring), `uuid` (`PersonId`), `serde`; on macOS, `block2` / `objc2` / `objc2-contacts` / `objc2-foundation` for the `CNContactStore` FFI in `address_book.rs`.

Notably it depends on **no other `openhuman` domain** — consistent with its "self-contained" docstring.

## Used by

- `src/core/all.rs` — registers the people controllers and schemas, and routes the `"people"` namespace.
- `src/openhuman/memory_store/` — reuses `people::types::{Person, PersonId, Handle}` (e.g. `Person` aliased as `Contact` in `kinds.rs`, and in `traits.rs`).

## Notes / gotchas

- **Cross-source merge safety (issue #1538):** two identities that share only a display name or only an unverified handle from different sources are **never** auto-merged. Merging only happens via explicit `link()`. Resolver tests lock this contract in.
- **Idempotent seeding:** `seed_from_address_book` re-runs as a no-op for already-known handles; on `PermissionDenied` it writes nothing (no partial state). The "primary" link target is first email, else first phone, else display name.
- **macOS Address Book FFI must not run on the main thread** — `CNContactStore` access requests deadlock there; `request_access` blocks on a completion-handler channel. Non-mac builds return an empty contact list.
- **Scoring constants are module-level**, not config-driven yet — kept fixed so tests stay stable; the docstring notes they can move to config later without breaking the API.
- **Composite is a product:** any zero component (e.g. a one-sided conversation → reciprocity 0) zeroes the whole score.
- **SQL runs on `spawn_blocking`:** the connection is sync `rusqlite` behind `Arc<Mutex<Connection>>`; `JoinError`s from blocking tasks are mapped into a synthetic `rusqlite` IO error.
- **Tests bypass the global store:** they construct `PeopleStore::open_in_memory()` and call `rpc::*` / `HandleResolver` directly rather than going through the schema adapters (which require the `OnceCell`-initialized global).
