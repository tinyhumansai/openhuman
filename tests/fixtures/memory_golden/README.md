# Golden memory-workspace fixture

**Generated, do not hand-edit.** Regenerate with
`scripts/regen-memory-golden-fixture.sh`.

| | |
| --- | --- |
| Captured at commit | `6996fa6bae2b35d81f2d3203a2a9f875dce34eda` |
| Captured on | 2026-09-01T07:35:46Z |
| Generator | `regenerate_golden_fixture` in `tests/memory_golden_fixture_e2e.rs` |
| Seeder | `openhuman_core::memory::store::golden::seed` |

## Contents

[`workspace/**.db`](./workspace) is a real memory workspace seeded through production write
paths and then `VACUUM`ed with `PRAGMA wal_checkpoint(TRUNCATE)`, so each
file is self-contained (no `-wal` / `-shm` siblings). It holds:

- documents in two namespaces (`golden-primary`, `golden-secondary`)
- both KV scopes (global and namespace)
- a graph triple
- an episodic row (which materialises the `episodic_fts` shadow tables and
  the sync triggers)
- a sealed, summarised conversation segment with both embedding tiers
- an event row (materialising `event_fts`) with a per-model embedding
- a `user_profile` facet
- a tinycortex leaf chunk with an embedding, and a summary tree sealed to an
  L1 summary node with its own embedding

[`manifest.txt`](./manifest.txt) is **derived from those DB files**, never written by hand.

## Why this is committed as a binary

[`.gitattributes`](../../../.gitattributes) marks `tests/fixtures/memory_golden/**/*.db binary`. Without
it the repo-wide `* text=auto eol=lf` rule would rewrite byte sequences inside
the blobs on checkout and corrupt them.

## Reviewing a change to this directory

A diff that touches this directory is a **schema-migration review**, not a
test-data refresh. Ask for the migration that carries existing user workspaces
across, and check that the manifest diff matches the DDL diff.

## See also

- [`tests/README.md`](../../README.md), the layout of the Rust test targets.
- [`memory` domain README](../../../crates/openhuman-core/src/memory/README.md) and [Memory architecture](../../../docs/gitbooks/en/developing/architecture/memory.md).
- [`vendor/tinymemory/`](../../../vendor/tinymemory), which owns the engine contracts and migrations.
