# Knip Cleanup Design

## Goal

Remove unreachable frontend files, unused code, and unused package dependencies
from the OpenHuman app without changing user-visible behavior or deleting
framework, test, desktop, or tooling entry points that are loaded indirectly.

## Scope

This cleanup is limited to the `openhuman` repository and the
`openhuman-app` workspace under `app/`. Rust code, vendored repositories,
generated assets, other umbrella submodules, and unrelated refactors are out of
scope.

The initial `pnpm knip` baseline reports:

- 34 unused files
- 2 unused dependencies
- 10 unlisted dependencies
- 239 unused value exports
- additional unused exported types and enum members
- stale or redundant entries in `app/knip.json`

These findings are candidates, not deletion instructions. Knip cannot always
see Tauri, Vite, Vitest, WebdriverIO, package-script, or convention-based entry
points.

## Approach

### 1. Make the analysis trustworthy

Update `app/knip.json` and package declarations so Knip knows about every real
application, test, desktop, and tooling entry point. Replace transitive
dependency reliance with direct declarations where source files import a
package. Remove stale ignores and patterns only when Knip can discover the
corresponding usage without them.

Configuration changes must be narrow. Broad ignore globs added solely to
silence findings are not acceptable.

### 2. Verify candidates independently

For each reported file or symbol:

1. Search the repository for static references, re-exports, scripts, generated
   manifests, test configuration, and string-based loading.
2. Check whether a framework or desktop runtime discovers it by convention.
3. Classify it as removable, internally used but over-exported, or deliberately
   public.

Files are deleted only when they have no runtime, build, test, documentation, or
tooling role. Used declarations with unnecessary exports become module-private
instead of being deleted.

### 3. Remove dead surface in small slices

Apply the cleanup in independently verifiable groups:

1. Knip configuration and dependency declaration corrections.
2. Unreachable files and dependencies.
3. Unused value exports and local declarations.
4. Unused exported types and enum members where removal is source-compatible
   inside the repository.

Each validated group is committed separately with `atomic-commit`, listing
every touched file explicitly.

### 4. Treat public contracts conservatively

Exports used only by tests may remain exported when they are intentional test
seams. Package-facing APIs, Tauri integration boundaries, generated-code
inputs, and documented extension points remain unless repository evidence
shows they are private and unused.

Any intentional finding that cannot be modeled as an entry point will receive
the narrowest possible Knip exception and a short reason.

## Validation

After each cleanup group:

- run Knip and confirm the targeted findings disappear without new findings;
- run TypeScript compilation;
- run affected Vitest tests;
- run lint and formatting checks for touched files.

Before completion:

- `pnpm knip`
- `pnpm typecheck`
- `pnpm test`
- `pnpm lint`
- `pnpm format:check`
- `pnpm build`

The cleanup is complete when Knip has no unexplained actionable findings, all
quality gates pass, and the diff contains no behavior changes beyond removal of
unreachable code and unnecessary export visibility.

## Non-goals

- Redesigning features or module boundaries
- Changing UI behavior
- Removing Rust code
- Updating unrelated dependencies
- Publishing branches or opening pull requests
