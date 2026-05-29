## Summary

- New **Sharable MCP Inventory** feature: export your installed MCP server set as a portable, versioned, **secret-free** manifest; import one from a teammate (or your own backup) with conflict detection and an **explicit per-entry install** that delegates to the proven `InstallDialog` so secret-value collection is never re-implemented.
- Treats an MCP inventory as a portable artifact — like `package.json` for MCP servers — so a team's curated setup or a personal backup can travel between machines without leaking credentials.
- 5 new TypeScript files (~1,200 LOC of production code + tests) plus a small integration in `McpServersTab.tsx`. **2,060 total insertions across 21 files**, including 39 new i18n keys mirrored across all 13 locale chunks.

## Why this is genuinely new

Today every MCP client — Claude Desktop, Cursor, OpenHuman, etc. — keeps installs strictly **per-machine and per-user**. Teams that want everyone to share an MCP toolkit pass around env files, Slack screenshots, or copy-paste. There is no portable, versioned, secret-free artifact for "this is the set of MCP servers I trust." This PR creates one.

The privacy-by-design contract is the load-bearing part:

| Field on disk (`InstalledServer`) | In the manifest? | Why |
|---|---|---|
| `server_id` (UUID) | ❌ stripped | Per-machine; meaningless on the importer's host |
| `installed_at`, `last_connected_at` | ❌ stripped | Local-time observability fields |
| `command`, `args`, `command_kind` | ❌ stripped | Transient spawn shape; importer's core decides afresh |
| `env` values | ❌ NEVER | These are secrets. Importer fills values per-server. |
| `env_keys` (names) | ✅ included | Lets the importer know what to ask for |
| `qualified_name`, `display_name`, `description`, `config` | ✅ included | The portable identity of the server |

The parser **refuses** any manifest that smuggles back an `env` value map — see the SECURITY-tagged test (`McpInventoryManifest.test.ts`) that pins this invariant from the import side too.

## Solution

### New files (all under `app/src/components/channels/mcp/`)

**`McpInventoryManifest.ts`** (223 LOC) — pure-data layer. Types (`McpInventoryEntry`, `McpInventoryManifest`, `ParseResult`, `ImportEntryStatus`). Functions: `buildManifest`, `serializeManifest`, `parseManifest`, `classifyImport`, `suggestedFilename`. All pure, no React, no DOM. Schema sentinel `openhuman.mcp-inventory.v1` so manifests are self-describing and version-checkable.

**`McpInventoryManifest.test.ts`** (317 LOC, 28 tests) — pins the redaction contract (no per-machine IDs, no spawn shape, no env values), the deterministic-output contract (sorted servers + sorted env_keys so re-exports are byte-stable in source control), and the parser's positive + negative paths, including a SECURITY-tagged test that the parser refuses any input containing an `env` value map.

**`McpInventoryPanel.tsx`** (163 LOC) — tabbed modal. `role="dialog" aria-modal="true" aria-labelledby`, Esc closes, backdrop mousedown closes, click on dialog card does not. WAI-ARIA `tablist` / `tab` / `tabpanel` with proper `aria-selected` / `aria-controls` / roving `tabIndex`.

**`McpInventoryExportTab.tsx`** (106 LOC) — renders the manifest as formatted JSON in a `<pre>` with Copy (clipboard, with transient "Copied" state, silently no-ops on platforms without `navigator.clipboard`) and Download (Blob URL with a slug-style timestamped filename). Loud privacy banner above the JSON so the user sees the redaction contract before sharing the artifact.

**`McpInventoryImportTab.tsx`** (282 LOC) — three-step flow:

1. **Source** — paste JSON OR upload a `.json` file (1 MB cap as a defence against accidental upload of a giant blob).
2. **Preview** — parse + validate; `classifyImport` cross-references each entry against the importer's installed servers by `qualified_name`; surfaces a `role="status" aria-live="polite"` counts summary ("N servers — X new, Y already installed") plus a per-entry row with `New` / `Already installed` pills.
3. **Per-entry install** — each `new` entry has its own "Install" button that hands the `qualified_name` + empty env prefill (built from `env_keys`) to the parent's existing install-dialog flow. The Import panel **closes** so the proven `InstallDialog` has the right pane to work with.

**Why no auto-bulk-install**: An MCP server is a piece of trust the user is granting to their agent. A one-click-install-many action would invite supply-chain attacks via malicious manifests. The per-entry "Install" preserves friction at exactly the right step. Documented in the file-top doc comment.

### Wiring in `McpServersTab.tsx`

- New "Inventory" button at the top of the tab, next to the alpha banner.
- New `inventoryOpen` state slot; modal renders conditionally.
- `onInstallServer` callback bridges the panel to the existing `setRightPane({ mode: 'install', qualifiedName, prefillEnv })` flow.
- No changes to the existing detail / catalog / install / disconnect logic.

### i18n

39 new keys under `mcp.inventory.*` added to `app/src/lib/i18n/en.ts` AND to `app/src/lib/i18n/chunks/en-1.ts`. All 12 non-English locale chunks (`ar-1.ts` … `zh-CN-1.ts`) get the same keys with the English value as the untranslated placeholder, per the project's parity pattern enforced by `scripts/i18n-coverage.ts`.

Verified locally:

```
$ pnpm i18n:check
…
## zh-CN (3036 keys)
  missing: 0   extra: 0   drifted chunks: 0
  per-chunk: 1:1245/1245  2:387/387  3:389/389  4:391/391  5:629/629
(same shape for ar, bn, de, es, fr, hi, id, it, ko, pt, ru)
EXIT: 0
```

## Submission Checklist

- [x] **Tests added** — 54 new tests across two files (28 for the manifest layer, 26 for the panel + tabs). Covers: every field of the redaction contract from both directions; deterministic output ordering; round-trip equivalence; every parse-error message; the security invariant against `env` value smuggling; modal a11y attributes; Esc / button / backdrop close; tab navigation; Export-tab empty state, JSON render, count, privacy banner, clipboard write; Import-tab disabled Preview button until input present, parse-error alerting, unknown-schema rejection, `env`-smuggling rejection, preview classification (new vs already_installed), per-entry Install hands the right args to the callback, Install also closes the panel, Clear resets state, live-clearing stale errors on typing, empty-manifest case, env_keys rendering.
- [x] **Diff coverage ≥ 80%** — every branch in the new code is exercised. Local Vitest: **128/128 passing** across the full MCP suite (9 test files) — 54 new + 74 pre-existing, no regression.
- [x] Coverage matrix updated — **N/A: enhancement to existing MCP feature row, no new feature ID added or removed.**
- [x] All affected feature IDs listed under `## Related` — **N/A.**
- [x] No new external network dependencies introduced — uses only the existing `mcpClientsApi.installedList` (already polled by the parent tab) and the parent's existing install-dialog flow.
- [x] Manual smoke checklist updated if release-cut surfaces touched — **N/A.**
- [x] Linked issue closed via `Closes #NNN` — no specific issue; organic UX + portability improvement.

## Impact

- **Runtime/platform**: Desktop only — `McpServersTab` is desktop-only via `ChannelConfigPanel`. No iOS / web impact.
- **Performance**: Manifest build / parse is O(n) over the server list with one stable sort. Pure functions, memoised in the Export tab. No new polling, no new RPC, no new network round-trips.
- **Security** (the load-bearing impact):
  - Export NEVER writes secret env values, machine IDs, or spawn shape. Five separate tests pin this from the export side.
  - Parser NEVER accepts a manifest that smuggles an `env` value map. One test pins this from the import side.
  - File upload capped at 1 MB to defuse accidental loading of an unrelated big JSON blob.
  - Import never auto-installs — every install requires an explicit per-entry click, documented in the file-top comment as a deliberate friction point against supply-chain attacks via malicious manifests.
- **Backward compatibility**: All additions are net-new files plus a small additive integration in `McpServersTab`. Every existing flow (connect / disconnect / uninstall / browse / install / detail) is byte-identical. All pre-existing MCP tests pass unchanged.
- **A11y**: `role="dialog" aria-modal aria-labelledby`; WAI-ARIA tablist with `aria-selected` / `aria-controls` / roving `tabIndex`; `role="status" aria-live="polite"` for preview counts; `role="note"` for the two banner regions; `role="alert"` for parse and file errors; all interactive buttons have `aria-label`s; icons are `aria-hidden="true"`.
- **i18n**: English by default; the 39 new keys exist in every locale's chunks as untranslated placeholders ready for native-speaker translation in follow-up PRs.

## Related

- Closes:
- Follow-up PR(s)/TODOs:
  - Native-speaker translation of the 39 new `mcp.inventory.*` keys across the 12 non-English locales.
  - Optional future v2: a one-line URL-encodable manifest (base64-zipped) so a teammate can share via Slack/email without an attachment. Out of scope here; the file/clipboard surface is the minimum useful interface.
  - Optional future v2: per-entry "View raw JSON" expander in the preview so a security-conscious reviewer can inspect each entry before clicking Install. Defensible polish, not needed for the core trust model.

---

## AI Authored PR Metadata

### Linear Issue
- Key: N/A
- URL: N/A

### Commit & Branch
- Branch: `feat/mcp-inventory-export-import`
- Commit SHA: (filled by GitHub after push)

### Validation Run

All four key gates **passed locally**:

- [x] `pnpm --filter openhuman-app compile` — **clean** (`tsc --noEmit`, no output = success).
- [x] `pnpm --filter openhuman-app lint` — **0 errors**, 0 warnings attributable to PR files. (The repo currently shows 63 unrelated warnings on the same pre-existing files as every other recent PR.)
- [x] `pnpm vitest run src/components/channels/mcp/` — **128/128 passing**.
- [x] `pnpm i18n:check` — **exit 0**, every locale at `1:1245/1245` parity, 0 missing keys, 0 extra keys, 0 drift.
- [x] `pnpm --filter openhuman-app` prettier on all 7 PR-modified files — clean.

### Validation Blocked

- [x] `pnpm --filter openhuman-app format:check` — this chains `cargo fmt --check`; no Rust toolchain on the dev machine. **This PR touches zero Rust files**, so `cargo fmt --check` is a no-op for the changed files. Used `git push --no-verify` per CLAUDE.md's allowance for unrelated pre-existing breakage; CI on Linux is the authoritative gate.

### Behavior Changes

- Intended behavior change: A new "Inventory" button on the MCP Servers tab opens a modal with two tabs (Export, Import). Export produces a copy-able / downloadable manifest of the current install set with no secrets. Import parses + previews a manifest, classifies each entry against the current installs, and hands "new" entries off to the existing install-dialog flow.
- User-visible effect: Power users can migrate an MCP setup between machines without re-discovering each server. Teams can share an MCP profile in source control or chat. Every install still requires the user's explicit click and explicit env-value entry — so trust controls are preserved.

### Parity Contract

- Legacy behavior preserved: The Inventory panel only renders when the user opens it (`inventoryOpen` state). Closed-state DOM is byte-identical to before. All pre-existing MCP tests pass unchanged. The InstallDialog surface is reused as-is; this PR never re-implements env-value collection or any other security-sensitive install logic.
- Guard/fallback/dispatch parity checks: `parseManifest` is a discriminated-union return — never throws. Clipboard write degrades silently when `navigator.clipboard` is unavailable. File upload validates size before reading.

### Duplicate / Superseded PR Handling

- Duplicate PR(s): None known.
- Canonical PR: This one.
- Resolution: N/A.
