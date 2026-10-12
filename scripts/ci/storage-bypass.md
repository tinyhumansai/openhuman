# Storage bypass coverage and retained debt

The cancellation-supervised checks lane scans every immediate `crates/*/src`
host directory. The original core document/storage implementation exclusion and
sibling test exclusions remain. It also scans explicitly curated classic
TinyAgents session/task/completion stores, TinySkills catalog storage, and the TinyMCP registry/audit stores actually opened
by host composition. Native storage driver implementations are not bypasses;
their host adapters remain scanned.

Rules cover database opens, filesystem writes/creation, rename/copy/hard-link
publication, tempfile builders, JSON writer destinations, the known filesystem
helper definitions `write_private`, `create_private`, and `write_atomic`, and
store open calls with a nearby `.db` or `.jsonl` literal argument, file-backed `File*Store::new` / `File*Ledger::new` constructors, and the JSONL task opener. Tokio calls
are included: the original `fs::write` pattern already recognized their `fs::`
suffix. The missing embed detection was a directory coverage bug.

## Two exact debt inventories

`storage-bypass-baseline.json` retains the original **68** core rule identities.
This change does not increase or replace that baseline. Regeneration requires an
existing baseline and refuses new findings; it can only remove fixed sites.
The merge-base check also prevents editing the manifest to accept new debt.

`storage-bypass-extension.json` contains **109** reviewed identities for coverage
that this change introduces. Each identifies its exact rule, path, source line,
and occurrence, with a destination classification, reason, and owning issue.
These are sites, not 109 independent stores. The initial extension must match
source already present at the canonical merge base, including its recorded
vendor gitlinks. A branch-added or moved writer cannot enter this inventory.
Once the extension exists in the base, its identities can only shrink. Removing
an entry while its writer survives fails the ordinary added-finding check.

`storage-bypass-allow-reasons.json` audits the original **16** pinned exclusions
and one existing browser lock permission probe. The old claim that the port has
no blob store is removed: local artifact reservation is historical migration
debt, not an absent native BlobStore capability. Initial audited exclusions must
also exist at the canonical merge base; later exclusions cannot grow. An exact
site allowance never covers a second occurrence or another call in the file.

The browser lock probe opens an existing file without create, truncate or a
write call. The separate Apple chat.db record explicitly uses
`SQLITE_OPEN_READ_ONLY`. Neither classification establishes storage authority.
Other records distinguish classic compatibility/import paths, machine bootstrap,
scratch, explicit external exports, and genuine unmigrated state. A classification
is review evidence, not a claim that its issue or native host routing is complete.

## Bounds and verification

This is a source guard, not Rust name resolution or whole-program dataflow.
It does not expand macros or resolve renamed imports, follow arbitrary writer
helpers, or infer a destination behind a variable. Opener literal matching spans
the current and following source line; curated owner store definitions cover
known indirect host opens. New rules mask Rust comments and quoted fixtures;
original core rule identities retain their existing lexical behavior.

Real CI uses the canonical merge base and fails if the base, original baseline,
or recorded vendor source is unavailable. Current curated vendor files must also be readable, and curated directories must contain Rust source; absent or empty checkouts cannot count as debt removal. `--root` also checks history when it names a real Git checkout; it supports isolated source
fixtures without Git; those tests independently exercise the manifest comparison policy.
Do not use a synthetic fixture result as proof of actual host storage isolation.

The embed Tokio writer and five additional coverage/regeneration cases genuinely
failed against the original scanner (0 passed, 6 failed), then passed unchanged.
A separate missing-baseline regeneration case also genuinely failed before its
fix. Current focused scanner/debt/lane controls cover exact identity moves and
copies, initial branch writes, later manifest growth, surviving unrecorded writes,
missing history, absent/empty curated source, duplicate core records, comments/fixtures, original core debt, and both CI profiles. The review controls reproduced duplicate-baseline and missing-source failures (0 passed, 2 failed), empty-source failure (0 passed, 1 failed), and missing scripts path-filter coverage (0 passed, 1 failed) before their fixes. The initial focused suite had 60 passing controls. A subsequent review reproduced two real symlink-root bypasses (0 passed, 2 failed): missing curated source was ignored, and `--root` skipped required canonical history. Shared canonical-path checkout detection fixes both; the expanded suite has 62 passing controls. Checker, debt helper, and policy-only JSON/Markdown changes arm the scripts self-test lane.

Run:

```sh
node --test scripts/__tests__/storage-bypass*.test.mjs scripts/__tests__/self-hosted-lanes.test.mjs
bash scripts/ci-cancel-aware.sh node scripts/ci/check-storage-bypass.mjs
```

This guard does not implement backend migration, desktop import authority, or
Memory/SQLite/MongoDB runtime composition. Those belong to the linked owner
issues and their authentic native integration proofs.

The latest review added task/completion and catalog owner coverage. Three curated/constructor regressions and one host task-opener regression failed before their fixes. The expanded focused suite passed 70 controls. The additional 14 canonical sites include a read-only catalog opener, refetchable catalog cache publication, classic task/completion logs, and classic file-backed status/import stores. No site is classified as migrated solely because another native implementation exists. Changes to the shared Rust comment masker also arm the scripts lane.
