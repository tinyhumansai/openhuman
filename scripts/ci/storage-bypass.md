# Storage bypass coverage and retained debt

The cancellation-supervised checks lane scans every immediate `crates/*/src`
host directory. The core storage implementation and sibling test exclusions
remain. Curated sources also cover TinyAgents session/task/completion owners,
TinySkills catalog storage, and TinyMCP registry/audit stores opened by host
composition. Native storage backend implementations are outside this scan;
their host adapters remain scanned.

Rules detect database opens, file mutations, tempfile builders, JSON writers,
known filesystem writer helpers, store openers with a nearby `.db` or `.jsonl`
literal, file-backed `File*Store::new` / `File*Ledger::new` constructors, and the
JSONL task opener. Tokio calls are included. The former embed gap was caused by
scanning only the core directory.

## Exact inventories

`storage-bypass-baseline.json` contains **39** remaining original core rule
identities after deleting stale and retired entries. Regeneration requires an
existing baseline, refuses new findings, and can only remove fixed sites.
Canonical merge-base validation also rejects baseline growth.

`storage-bypass-extension.json` contains **51** reviewed identities for the
broader coverage. Each records its rule, path, source line, occurrence,
classification, reason, and owning issue. These are identities, not independent
stores or completed migrations. The initial extension must already exist at the
canonical merge base and its recorded vendor gitlinks. A writer added, moved,
or copied by the PR cannot enter the inventory. Once established in canonical
main, this manifest can only shrink.

The merge with current main removes 53 retired extension identities and reviews
11 existing canonical identities: artifact file materialization, legacy memory
import/layout checkpoints, a test-only migration fixture, source-preserving MCP
import copies, legacy tinymemes document retirement, and downloaded file bytes.
The thread facade and turn-state mirror replace their removed classic store
paths in the curated source list, preserving coverage of the current owners.

`storage-bypass-allow-reasons.json` audits **15** current canonical exclusions.
Retired file-store fallbacks are not restored. Exclusions cover read-only legacy
imports over scratch copies, machine/bootstrap state, user-visible artifact
files and derived views, and an exact test-only classic transcript fixture.
Artifact files do not imply that native BlobStore support is absent. Initial
exclusions must exist in canonical source; later exclusions cannot grow. An
allowance never covers a second occurrence or another call in the same file.

`storage-bypass-relocations.json` records exactly **7** credential fallback
sites whose tracked source was lost during the `8efcf6b` baseline refresh and
later appeared under `security/credentials/`. The checker pins this identity
set, verifies each site's source at `5f01951c`, and verifies the four earlier
`json-write` debt records against the baseline immediately before the refresh.
The three `file-mutation` identities have historical source proof, including
the auth-profile temporary-file rename, but do not inflate the old writer
baseline. Every record links to [OpenHuman #7421](https://github.com/tinyhumansai/openhuman/issues/7421),
which tracks the required key-source and legacy desktop import work. Missing,
stale, copied, or arbitrary records fail; ordinary writers still pass only
through the no-growth baseline or extension ratchets.

Classifications are review evidence. They do not establish tenant authority,
cloud isolation, migration completion, or an export's admission policy.

## Bounds and validation

This is a source guard, not Rust name resolution or callgraph/dataflow analysis.
It does not expand macros, resolve arbitrary import aliases, or infer a writer's
destination from a variable. Literal opener matching spans the current and next
source line. New rules mask Rust comments and quoted fixtures; original core
identities retain their existing lexical behavior.

Real checkouts require canonical history, the original baseline, and recorded
vendor sources. Missing/empty curated roots, invalid/duplicate records, and stale
allowances fail closed. `--root` enforces history for real checkouts, including
symlink spellings; isolated fixtures exercise source and debt policy separately.
A fixture pass is not proof of host storage isolation.

The focused scanner/debt/lane suite covers initial branch writers,
copied/moved identities, later growth, missing history and baselines,
absent/empty curated roots, symlink spellings, comments/fixtures, current owner
paths, cancellation supervision in both CI profiles, and exact credential
relocation source proofs. Policy JSON/Markdown, scanner, debt-helper, and shared
comment-masker changes arm the scripts regression lane.

```sh
node --test scripts/__tests__/storage-bypass*.test.mjs scripts/__tests__/self-hosted-lanes.test.mjs
bash scripts/ci-cancel-aware.sh node scripts/ci/check-storage-bypass.mjs
```

Backend migration, desktop import authority, and native runtime composition
remain the responsibilities of their linked owner issues and integration tests.
