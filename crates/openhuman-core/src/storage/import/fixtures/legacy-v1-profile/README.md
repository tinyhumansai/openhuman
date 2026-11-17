# Legacy v1 profile storage fixture

`storage.db` is a semantic profile fixture in the physical SQLite format of
the historical public `tinystoragedrivers` v1 API at
`0c6dcfce57201fadd830431f36fd759c3baa897a`. The standalone generator writes it
through that API's `StorageBackend::for_scope`, `DocumentStore`, and
`put_with` methods; it does not create or edit TSD tables directly.

The generator's serializable wire structs mirror the historical OpenHuman
owner models and writer transformations from `2d3e22636e^`:

- `platform/cost/types.rs::CostRecord` and `TokenUsage` are written to
  `cost_records`, including the owner timestamp and `ts_ms`.
- `agent/artifacts/types.rs::ArtifactMeta` is written to `artifacts` with the
  old writer's `created_at` to `created_ms` conversion; its
  `artifact_args` document uses the old `{ "args": ... }` wrapper.
- `profiles/types.rs::ProfileMeta` is written to the `cluster` scope's
  `profiles` collection with `created_at` removed from the body and retained
  as TSD `created_ms` metadata, matching `ProfileRegistry::body_of`.
- The isolated desktop profile database stores owner documents in the
  historical `local` scope; the profile directory itself supplied user
  isolation. The fixture writes an `approvals` v1 request with RFC3339
  timestamps and a `threads_transcript_keys` record keyed by raw thread id.
  The approvals and threads owners migrate those into their current
  collections and ids before their public APIs read them.
- The thread owner also has a committed thread and message in the historical
  SQLite database. These are TinyAgents' actual v1 inputs: a
  `conversation_threads_v1` row keyed by the thread id's SHA-256 and a
  `conversation_messages_v1:<sha>:<incarnation>` stream whose entries carry
  the v1 `schema`, `incarnation`, and camelCase `thread` / `message` payloads.
  The boot schema migration converts them to the current conversation
  collections; the fixture test reads the thread and message through the
  current typed owner API after migration.
- The TinyFlows catalog fixture uses its historical `flows_definitions`
  collection with `graph_json` and RFC3339 timestamps. The catalog's schema
  migration converts it, and the public `flows_list` operation must return it.
- The secret uses TinyStorage's public `DocumentSecrets::set` writer against
  the historical v1 backend, then the integration test reads it with
  `DocumentSecrets` after desktop boot.
- `secrets` contains an encrypted `fixture-secret` document in the same scope
  using the fixture-only master key `[0x5a; 32]`; the integration test reads it
  with TinyStorage's `DocumentSecrets` API after desktop boot.

The host E2E test copies this database to an existing profile's
`users/u1/workspace/storage/storage.db`, boots the desktop host with layout
auto-migration enabled, then restores the migration archive to a separate
temporary directory, verifies the archive includes the original storage path,
and opens its database. It reads thread messages and transcript key, approvals,
secrets, and flows from the migrated profile through their owner APIs. It then
deletes the flow, reboots the desktop host, and checks that boot does not
resurrect the row from the archived database. The focused core import test
continues to verify the owner schema conversions for cost, artifacts, and
profile metadata, as well as non-resurrection after deletion.

The standalone generator is under `generator/`; it is not a workspace member
or product dependency. Regenerate a fresh database at a caller-selected path:

```sh
cargo run --locked --manifest-path crates/openhuman-core/src/storage/import/fixtures/legacy-v1-profile/generator/Cargo.toml -- /absolute/path/to/storage.db
```

The fixture is semantically reproducible. Its SQLite bytes can differ between
generations because `DocumentSecrets` encrypts the secret with randomized
ciphertext; verify fixture updates by running the focused fixture test rather
than comparing a fixed whole-file SHA-256.
