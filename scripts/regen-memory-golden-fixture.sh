#!/usr/bin/env bash
#
# Regenerate the committed golden memory-workspace fixture.
#
#   tests/fixtures/memory_golden/workspace/**.db   the captured workspace
#   tests/fixtures/memory_golden/manifest.txt      schema manifest DERIVED from it
#
# Both are produced together by the `#[ignore]`d `regenerate_golden_fixture`
# test in `tests/memory_golden_fixture_e2e.rs`, driven by the CURRENT build.
#
# ── When to run this ─────────────────────────────────────────────────────────
#
# Only when you are deliberately changing the memory store's on-disk schema,
# and only alongside a migration for existing user workspaces. Regenerating
# the fixture is how you re-baseline the gate; it is NOT how you fix a
# failing test. If `memory_golden_fixture_e2e` failed and you did not intend
# a schema change, the failure is the bug.
#
# ── The ordering rule this script exists to protect ──────────────────────────
#
# The manifest is never hand-written: it is dumped from the fixture DB files.
# So editing a `CREATE TABLE` and editing `manifest.txt` to match still fails,
# because the committed `.db` was written by the older binary. Getting back to
# green requires running THIS script, which rewrites the binary blobs — a
# change a reviewer can see.
#
# Usage:
#   scripts/regen-memory-golden-fixture.sh
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

FIXTURE_DIR="tests/fixtures/memory_golden"
README="$FIXTURE_DIR/README.md"

echo "[golden-fixture] regenerating from the current build"
SOURCE_SHA="$(git rev-parse HEAD)"
if ! git diff --quiet -- src/openhuman/memory; then
  echo "[golden-fixture] WARNING: src/openhuman/memory has uncommitted changes." >&2
  echo "[golden-fixture]          The fixture will capture the WORKING TREE, but the" >&2
  echo "[golden-fixture]          README will record ${SOURCE_SHA}. Commit first." >&2
fi

GGML_NATIVE="${GGML_NATIVE:-OFF}" cargo test \
  --manifest-path Cargo.toml \
  --test memory_golden_fixture_e2e \
  regenerate_golden_fixture -- --exact --ignored --nocapture

# Record provenance next to the blobs. The SHA is the whole point: it is what
# makes "this fixture predates the change under review" checkable.
cat > "$README" <<EOF
# Golden memory-workspace fixture

**Generated — do not hand-edit.** Regenerate with
\`scripts/regen-memory-golden-fixture.sh\`.

| | |
| --- | --- |
| Captured at commit | \`${SOURCE_SHA}\` |
| Captured on | $(date -u +"%Y-%m-%dT%H:%M:%SZ") |
| Generator | \`regenerate_golden_fixture\` in \`tests/memory_golden_fixture_e2e.rs\` |
| Seeder | \`openhuman_core::openhuman::memory::store::golden::seed\` |

## Contents

\`workspace/**.db\` is a real memory workspace seeded through production write
paths and then \`VACUUM\`ed with \`PRAGMA wal_checkpoint(TRUNCATE)\`, so each
file is self-contained (no \`-wal\` / \`-shm\` siblings). It holds:

- documents in two namespaces (\`golden-primary\`, \`golden-secondary\`)
- both KV scopes (global and namespace)
- a graph triple
- an episodic row (which materialises the \`episodic_fts\` shadow tables and
  the sync triggers)
- a sealed, summarised conversation segment with both embedding tiers
- an event row (materialising \`event_fts\`) with a per-model embedding
- a \`user_profile\` facet
- a tinycortex leaf chunk with an embedding, and a summary tree sealed to an
  L1 summary node with its own embedding

\`manifest.txt\` is **derived from those DB files**, never written by hand.

## Why this is committed as a binary

\`.gitattributes\` marks \`tests/fixtures/memory_golden/**/*.db binary\`. Without
it the repo-wide \`* text=auto eol=lf\` rule would rewrite byte sequences inside
the blobs on checkout and corrupt them.

## Reviewing a change to this directory

A diff that touches this directory is a **schema-migration review**, not a
test-data refresh. Ask for the migration that carries existing user workspaces
across, and check that the manifest diff matches the DDL diff.
EOF

echo "[golden-fixture] wrote $README (source SHA ${SOURCE_SHA})"
echo "[golden-fixture] fixture size:"
du -sh "$FIXTURE_DIR"
git status --short -- "$FIXTURE_DIR"
