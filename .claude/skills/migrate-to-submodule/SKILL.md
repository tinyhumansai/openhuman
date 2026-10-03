---
name: migrate-to-submodule
description: Plan and execute moving non-host-specific code (and its tests) from the OpenHuman core (crates/openhuman-core) into the vendored tiny* submodule libraries (vendor/tiny*), then release the submodule and re-pin the registry digests. Use when asked to "move X out of core", "upstream this into tinyfoo", "extract a crate", or to shrink the host to composition/policy only.
---

# Migrate core code into a vendored submodule

OpenHuman is the **host**: composition, config, security policy, approvals,
lifecycle, and adapters. Behavior that is not specific to OpenHuman belongs in
the owning `vendor/tiny*` project (see "Submodule ownership" in `CLAUDE.md`).
This skill drives one migration end to end: plan, move code + tests, ship the
submodule release, re-pin the host.

Default to executing. Use plan mode only for step 1 (the split decision), and
only when the boundary is genuinely ambiguous or the move is large. Work in a
worktree (`worktree <slug>` from the repo root); never on `main`. Do not squash
commits; the auto-commit hook checkpoints for you.

## How TinyBus fits

TinyBus is the seam that makes this migration possible and shapes its design.

- A loadable module is a first-party `cdylib` speaking the TinyBus module ABI,
  downloaded from a pinned GitHub release and attached to an in-process broker
  as a bus peer. The core calls it through a **typed contract**, not a Rust
  import.
- Each module has a `*-bus` contract crate (names, methods, request/response
  types, contract version). It is synchronous and free of I/O and runtime deps.
  **Never redeclare a contract type in OpenHuman; call members through the
  contract constants.**
- Because the module is a separate binary in the same process, anything you move
  must be reachable from the host through the bus contract. So a migration is
  usually three pieces: (a) implementation in the module crate, (b) new/changed
  methods and types in the `*-bus` crate (bump its contract version), (c) a thin
  host adapter under `crates/openhuman-core/src/modules/<name>.rs` (or the
  domain that calls it) that forwards to the bus and keeps policy/config/creds.
- Data that cannot cross a boundary (task-local state, callbacks, secrets) is
  passed explicitly in the request, or served *to* the module as a host callback
  (see `modules/memory_host.rs`). Credentials never ride in tool arguments;
  they go through module init/reinit config.
- Module code must not depend on `openhuman`. Host callbacks are traits/bus
  services the module defines and the host implements.
- Not every submodule is a loadable module. Pure libraries (`tinyagents`,
  `tinytools`, `tinychannels`, `tinyskills`, `tinyvoice`, `tinywallet`, ...) are
  linked via Cargo; a move there needs no bus contract or digest re-pin, only a
  gitlink bump. Check the module's `MODULE.md`/README to know which kind it is.

## Step 1: Plan (freedom to decide the split)

1. Identify the code to move and read its callers. Use Explore/Grep on
   `crates/openhuman-core/src/<domain>/`.
2. Classify every item as **moves** or **stays**:

   | Moves to the submodule | Stays in OpenHuman |
   | --- | --- |
   | Algorithms, parsers, protocol/wire types, engines, generic tools, provider clients with no product identity | Config schema and loading, `SecurityPolicy`/approvals/sandbox decisions, credential resolution (`resolve_backend_credential`), `Outcome<T>` controllers and RPC schemas, event-bus publishing, product identity/attribution headers, Redux/Tauri glue |
   | Tests that exercise only the moved logic | Cross-module tests, host-adapter tests, JSON-RPC E2E, anything needing the mock backend or `CoreContext` |

3. Pick the destination using the ownership table in `CLAUDE.md`. Put the change
   in the repo that owns the behavior, not where it is easiest to land. Read the
   target's `README.md`, `AGENTS.md`, crate boundaries and `*-bus` crate first.
4. **You may create new crates** in the submodule workspace when the code has no
   good home (e.g. a new `tinyfoo-<thing>` library crate, or a new `*-bus` if a
   new module needs its own contract). Prefer extending an existing crate. Only
   propose a brand-new submodule/repo when no existing project owns the concept;
   that one is a user decision, so stop and ask.
5. Write the plan down briefly (in the PR description or `docs/plans/`): what
   moves, what stays, new crates, bus contract changes, dependency direction,
   and the release order. Skip a formal doc for a small, obvious move.

Dependency rules to check while planning: the core must not depend on
`tinyhumans-sdk` (only `openhuman-tinyhumans` may); a module must not depend on
`openhuman`; there is one `tinytools` copy (via `vendor/tinyagents/`); tool-call
parsing/dialects/agent loop belong in `tinyagents`, not the host.

## Step 2: Move the code, upstream first

Submodule PRs go **first**, raised against that repo's canonical upstream
(`tinyhumansai/<name>`), never a fork. Follow the submodule's own `AGENTS.md`/
`CONTRIBUTING.md` (fmt, clippy `-D warnings`, per-file coverage gates such as
90% in tinysearch's release workflow).

1. Branch inside the submodule at the recorded gitlink (the `worktree` helper
   already put every submodule on branch `<slug>`). Do not create nested
   worktrees.
2. Move the implementation into the target crate with `git mv`-style history
   where practical. Keep public API minimal and documented; keep files
   under about 500 lines.
3. Move tests with the code. Unit tests go beside the moved modules in the
   submodule. Add bus-contract tests in the `*-bus` crate for any new wire
   type. Tests that need the host stay in OpenHuman.
4. If a bus contract changed, bump its version and update both sides in the
   same change set. Keep the contract synchronous and I/O-free.
5. Replace the removed core code with the thin host adapter. Delete the old
   implementation and its old tests, do not leave a copy. Keep `Outcome<T>`
   controllers and RPC namespace strings (wire contracts) unchanged.
6. Verify in the submodule: `cargo fmt --all -- --check`,
   `cargo clippy --all-targets --all-features -- -D warnings`,
   `cargo test --all-features` (plus the coverage gate if the repo has one).
7. Open the submodule PR ready for review, babysit to green, merge.
   Unshallow submodule clones before merging upstream if needed.

If the change spans nested submodules (`tinyagents/vendor/tinytools`,
`tinymemory/vendor/tinycortex`, ...), release/merge the innermost first and bump
gitlinks outward.

## Step 3: Release the submodule

Only for **loadable modules** (a `cdylib` with a registry record). Pure Cargo
libraries just need the merged commit.

1. Trigger the submodule's `Release` workflow (`workflow_dispatch` on `main`,
   input `bump`: patch/minor/major/current), e.g.
   `gh workflow run release.yml -R tinyhumansai/<name> -f bump=patch`.
   It runs fmt/clippy/tests, builds per-platform archives, tags `vX.Y.Z`, and
   publishes a GitHub release with `checksum.toml`.
2. Wait for it to finish (`gh run watch`). Confirm the release has one archive
   per host key the registry lists (macOS 15/26 arm64+x86_64, Ubuntu 22.04/24.04
   x86_64+arm64, Windows) plus `checksum.toml`.
3. Ask before triggering a release if the user has not already authorized it: it
   publishes tags and artifacts and is hard to undo.

## Step 4: Re-pin the host

Modules are pinned **twice**, and both must describe the same release
(`scripts/ci/check-module-pins.mjs`):

1. **Submodule gitlink**: `git -C vendor/<name> fetch --tags && git -C vendor/<name> checkout vX.Y.Z`,
   then stage the gitlink in the superproject.
2. **Registry record**: edit `crates/openhuman-core/src/modules/registry/records_*.rs`
   for the module: `version`, `release_url`, and every `PlatformAsset`
   `archive` name and `sha256`.
   - Copy digests **verbatim from the published release's `checksum.toml`**
     (`gh release download vX.Y.Z -R tinyhumansai/<name> -p checksum.toml`).
   - **Never compute a digest from a local build.** A locally recomputed hash
     agrees with itself no matter what was served, which defeats the pin.
   - At load time tinybus fetches the release's own `checksum.toml`, compares it
     with the host pin, hashes the archive, then extracts. A re-cut tag stops
     matching instead of silently replacing what runs in-process.
3. Update anything else that names the version (for example the pinned
   archive version and sha256 in the CI workflows). If a module is deliberately off-tag, that needs an entry in
   `scripts/ci/module-pin-exemptions.json` with a reason. Prefer a fresh
   release over an exemption.
4. Bump the OpenHuman-side contract crate reference if the `*-bus` version moved,
   and update `crates/openhuman-core/src/modules/README.md` and
   `platform/about_app/` if the layout or user-visible capability changed.
5. Verify:
   ```bash
   node scripts/ci/check-module-pins.mjs
   node scripts/ci/check-submodule-monotonic.mjs
   pnpm rust:layout
   cargo check --manifest-path Cargo.toml
   cargo test -p openhuman-cli --test <affected>   # or: pnpm debug rust <filter>
   ```
   Test with a feature both enabled and disabled if a gate moved. Never export
   `CARGO_TARGET_DIR`. Run long CI-style commands through
   `scripts/ci-cancel-aware.sh`.

## Step 5: Open the OpenHuman PR

- PR against `upstream` (`tinyhumansai/openhuman`), ready for review, not draft.
  If the submodule release is not out yet, say so and mark draft only then.
- Body: what moved, what deliberately stayed and why, new crates, bus contract
  changes, links to the submodule PRs and release, and the digest source.
- The gitlink must point at a commit that exists upstream (merged, tagged).
- Hand off to `pr-babysitter` for CI and review threads.

## Checklist

- [ ] Every moved item is non-host-specific; host policy/config/creds stayed
- [ ] Tests moved with the code; cross-module/host tests stayed
- [ ] Bus contract updated and versioned; no redeclared contract types
- [ ] Submodule PR merged upstream first; fmt/clippy/tests/coverage green
- [ ] Release published with `checksum.toml` (loadable modules)
- [ ] Gitlink and registry `version`/`release_url`/digests agree, digests copied from the release
- [ ] `check-module-pins` and `check-submodule-monotonic` pass
- [ ] Old core code and tests deleted, docs updated
