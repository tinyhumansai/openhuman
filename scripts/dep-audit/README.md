# `scripts/dep-audit/`: Cargo dependency audit

Finds dependencies we can drop, unify, or slim across OpenHuman **and every
Cargo submodule under `vendor/`**, using
[`tinyanalyzer`](https://github.com/tinyhumansai/tinyanalyzer).

```bash
pnpm dep:audit                     # full sweep -> target/dep-audit/REPORT.md
pnpm dep:audit --snapshot          # ...and archive it as docs/dep-audit/<date>.md
pnpm dep:audit --targets '^(root|tinyagents)$' --top 25
```

A full sweep of 24 targets takes about 20 seconds; nothing is compiled, the
tool only runs `cargo metadata` and parses source.

The run avoids side effects on the tree: `cargo metadata` rewrites a
`Cargo.lock` that is stale relative to its manifest ([`crates/openhuman-app`](../../crates/openhuman-app/README.md)'s
lockfile in particular), or creates one where a target had none, so `run.sh`
records each target's lockfile state (present, with its exact contents, or
absent) before analyzing it and restores that state afterwards, restoring on
interruption too, printing which lockfiles it had to put back or remove. It
refuses to analyze a target whose `Cargo.lock` is a symlink rather than
backing up and writing through it. Refresh a lockfile deliberately (`cargo
update` / `cargo generate-lockfile`) if you want it refreshed.

## Files

| File | Role |
| --- | --- |
| [`run.sh`](./run.sh) | Discovers targets, runs `tinyanalyzer` once per target, then calls `report.mjs`. `--help` lists the flags. |
| [`report.mjs`](./report.mjs) | Folds the per-target JSON into `REPORT.md` and `summary.json`. Re-runnable on its own: `node scripts/dep-audit/report.mjs --reports target/dep-audit`. |
| [`tinyanalyzer.toml`](./tinyanalyzer.toml) | Shared analyzer config passed to every target (`--config`). Holds the `ignore_unused` list; see below before editing it. |
| [`../../docs/dep-audit/<date>.md`](../../docs/dep-audit/2026-09-19.md) | Committed snapshots from `--snapshot` runs, for diffing against the next run. |

## Prerequisites

- `tinyanalyzer` on `PATH` (`run.sh` prints the install one-liner if missing).
- Submodules checked out: `git submodule update --init --recursive vendor/`.
- Node 20+ (for `report.mjs`), `git`, `grep`.

## What gets analyzed

`run.sh` builds the target list itself, so a new submodule is picked up
automatically:

1. `root`, the OpenHuman workspace ([`Cargo.toml`](../../Cargo.toml) at the repo root).
2. `openhuman-app`, the Tauri host. It is `exclude`d from the root
   workspace and has its own `Cargo.lock`, so it is a separate graph.
3. Every entry of `git submodule status --recursive` that has a `Cargo.toml`,
   named after its directory (`tinyagents`, `tinybus`, `tinycortex`, …).

Nested checkouts (`vendor/tinymcp/vendor/tinybus`) are **skipped when a
checkout of the same repo at the same commit was already analyzed**, and
otherwise get a path-derived suffix (`tinybus@vendor_tinybox_vendor_tinybus`)
so both pins show up. A suffixed row in the report therefore *is* a finding:
that submodule pins a different commit of a shared dependency than its
siblings. `--keep-nested` analyzes every checkout regardless.

## Reading `REPORT.md`

### Targets

One row per target with its commit and headline counts. "Crates in graph" is
the `cargo metadata` resolve for every platform (no `--filter-platform`),
using each package's **default features** (tinyanalyzer does not pass
`--all-features`), which is why Windows-only crates appear on a Linux run,
why a duplicate listed here may not show in `cargo tree` on your host, and
why a crate reachable only through a non-default optional feature will not
appear at all.

### 1. Declared dependencies no source file names

tinyanalyzer's check is textual: a dependency is "unused" if no `.rs` file in
the package mentions it *the way Rust code references a dependency* , 
`dep_name::…`, `use dep_name`, `extern crate dep_name`, or `dep_name!` (`_`
for `-`; `dep_name` is the crate's rename alias when one is declared). That
misses crate names inside attributes (`#[tokio::test]`,
`#[derive(thiserror::Error)]`), so `report.mjs` re-checks every flag with a
grep over the package's own sources, **the crate's own directory,
recursively, plus the exact file (not directory) of every `[[test]]` /
`[[example]]` / `[[bench]]` / `[[bin]]` / build-script target declared by
`path =` in its `Cargo.toml`** (the root crate keeps its integration tests in
`tests/` that way; scanning the file rather than the shared `tests/`
directory keeps an unrelated sibling test from flipping the verdict).
Verdicts:

| Verdict | Meaning | Action |
| --- | --- | --- |
| **remove** | No `dep_name::…`, `use dep_name`, `extern crate dep_name`, `#[dep_name…` or `dep_name!` anywhere. | Delete the line, `cargo check` (both feature-on and feature-off builds if it was `optional`), delete the `dep:` feature if one existed. |
| **remove** (name only) | The bare word occurs in a comment or string but never as a path. | Same as above; the mention is not a use. |
| keep (attribute/macro path) | Used through an attribute or macro body. | Nothing. Listed so the tool's false positives stay visible. |

The re-check is a grep over raw text, not a syntax-aware scan: a comment or
string that happens to spell the exact reference pattern (`// dep_name::foo`)
is misclassified as a real use, same as tinyanalyzer's own check. Treat a
`keep` verdict as a strong signal, not a proof.

**Graph win** is the number of crates that leave the target's build if that
one line is deleted. It is `0 (kept by …)` when another package in the same
workspace still depends on the crate: the manifest gets cleaner, the build
does not get smaller. Sort your effort by graph win.

`ignore_unused` in `tinyanalyzer.toml` is empty and should generally stay
that way: it hides a crate from tinyanalyzer's own unused check in *every*
target, so a genuinely unused occurrence in some other target goes
unreported too. `report.mjs`'s own re-check already covers the false
positive this list historically existed for (`#[derive(thiserror::Error)]`
with no `use thiserror`) by scanning for the attribute form and reporting
"keep" instead of "remove". Only add an entry here for a crate that is
provably unreachable through any `use`/path/attribute/macro form
tinyanalyzer or the re-check could ever see.

### 2. Crates resolved at more than one version

Each version is compiled and linked separately. **Only the `root` (and
`openhuman-app`) sections cost the shipped build**; submodule sections show
where a requirement should move so the root can unify. Within a `root` /
`openhuman-app` section, a duplicate reached only through a `development`
dependency (see section 1's Kind column) costs test/CI build time, not the
shipped binary, check the Kind before treating a row as production weight.

"Pulled in via" names the *direct* dependencies whose subtree carries that
version, walked from tinyanalyzer's edge list. `direct dep of <pkg>` means one
of our own packages declares it. To unify:

- If one version's "via" list is a single old crate we control (a submodule
  or a direct dep with a stale requirement), bump it.
- If the old version is only reachable through an *unused* direct dependency
  (section 1), removing that dependency removes the duplicate too.
- If both versions are reached through third-party crates we do not control,
  check whether one of them has a newer release; otherwise accept it.

`cargo tree -i <crate>@<version>` in that target gives the full chain.

### 3. Heaviest direct dependencies

Per target, the direct dependencies with the largest **exclusive** transitive
footprint, crates that would leave the build entirely if this one were
dropped. "Reaches" is the raw transitive count, most of which something else
pulls in anyway. "Source" is checked-out source size, not binary size. Unlike
section 1, this table excludes a dependency whose only edge kind is
`development` (test/example/benchmark-only): those are never linked into the
shipped binary, so they do not belong in a shipped-build weight ranking.

A high exclusive count usually means default features pulling in a subtree we
do not use. Try `default-features = false` plus the two or three features
needed, then re-run the audit and compare the row. [`scripts/dep-sim.py`](../dep-sim.py) and
[`scripts/assert-shed.sh`](../assert-shed.sh) remain the tools for *proving* a reduction before
claiming it in a PR.

### 4. Version drift across repositories

Crates that two or more targets depend on directly but at
**semver-incompatible** versions (different major, or different minor for
`0.x`). Because the root workspace `[patch]`-es submodules in, every such row
becomes a duplicate in section 2 of `root`. Aligning the submodule's
requirement with the root's is usually a one-line change in that submodule
and removes a duplicate for free. Patch-level drift is counted but not
listed; cargo unifies it.

## Workflow for a clean-up pass

1. `pnpm dep:audit --snapshot` on a fresh branch.
2. Work section 1 by graph win, then section 4, then section 2's `root`
   rows. Submodule changes go to that submodule's own upstream as their own
   PR; bump the gitlink here afterwards (see the submodule PR conventions in
   `AGENTS.md`).
3. Re-run without `--snapshot` and diff `target/dep-audit/REPORT.md` against
   the committed snapshot. Commit a new snapshot with the clean-up PR.

## Extending

- `summary.json` next to `REPORT.md` has the same data as structured JSON
  (`targets[].unused`, `.duplicates`, `.heavy`, and top-level `drift`) for a
  future CI gate, e.g. failing on any new `remove` row with a graph win.
- Per-target `<name>.json` files are the raw tinyanalyzer reports and also
  carry file, complexity and dead-code findings that this report ignores.
  `tinyanalyzer vendor/<name>` opens the interactive dashboard over the same
  data.

## Further reading

- [`scripts/README.md`](../README.md) for the rest of the tooling, and [`scripts/ci/`](../ci/README.md) for the merge gates.
- [`docs/dep-audit/`](../../docs/dep-audit/2026-09-19.md), the committed snapshots.
- [`AGENTS.md`](../../AGENTS.md) for the `assert-shed.sh` / `dep-sim.py` rule before claiming a dependency reduction.
- [Building the Rust core](../../docs/gitbooks/en/developing/building-rust-core.md) and the [`vendor/`](../../vendor) submodules the audit walks.
