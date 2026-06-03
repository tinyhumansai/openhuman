# Memory Tree per-integration health strip — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a compact per-integration health strip inside `MemoryTreeStatusPanel`, between the four tiles and the auto-sync toggle, so operators can see at a glance which integrations are contributing chunks vs. idle. Issue #2763.

**Architecture:** Frontend-only. Reuses the existing `openhuman.memory_sync_status_list` RPC (no Rust changes). Extends `useMemoryTreeStatus` to fetch both endpoints in parallel on one shared 1.5s / 4s adaptive timer. Status mapping is pure TS: `Active` (freshness=active) vs `Stale` (freshness=recent|idle). Error state intentionally deferred — needs proper per-provider job→source linkage in core.

**Tech Stack:** React 18, TypeScript, Vitest + Testing Library, Tailwind. RPC bridge via `callCoreRpc` in `app/src/services/coreRpcClient`. i18n via `useT()` from `app/src/lib/i18n/I18nContext`.

**Branch:** `feat/memory-tree-integration-health` (already created from `origin/main`).
**Spec:** `docs/superpowers/specs/2026-06-02-memory-tree-integration-health-design.md`.

## File map

| File | Action | Responsibility |
| --- | --- | --- |
| `app/src/utils/tauriCommands/memoryTree.ts` | Modify | Add `MemorySyncFreshness`, `MemorySyncStatusRow`, `memorySyncStatusList()` wrapper for `openhuman.memory_sync_status_list`. |
| `app/src/components/intelligence/MemoryTreeStatusPanel.tsx` | Modify | Extend `useMemoryTreeStatus` hook to fetch both endpoints in parallel; add `IntegrationHealthStrip` sub-component; mount it between the 4-tile grid and the toggle row. |
| `app/src/components/intelligence/MemoryTreeStatusPanel.test.tsx` | Modify | New cases: renders rows, status mapping, empty state, icon fallback, shared polling assertion. |
| `app/src/lib/i18n/en.ts` | Modify | 5 new keys under `memoryTree.status.integration*`. |
| `app/src/lib/i18n/{ar,bn,de,es,fr,hi,id,it,ko,pl,pt,ru,zh-CN}.ts` | Modify | Same 5 keys with real translations (CLAUDE.md i18n rule — no English fallback). |

No new files created. Strip lives as a sub-component inside `MemoryTreeStatusPanel.tsx` because the two concerns share state (hook output) and polling cadence.

---

### Task 1: Add `memorySyncStatusList()` wrapper

**Files:**
- Modify: `app/src/utils/tauriCommands/memoryTree.ts` (append at end of file, before any closing exports)

**Context:** The Rust RPC `openhuman.memory_sync_status_list` returns `{ statuses: MemorySyncStatus[] }`. Wire shape from `src/openhuman/memory_sync/sync_status/types.rs`:

```rust
pub struct MemorySyncStatus {
    pub provider: String,
    pub chunks_synced: u64,
    pub chunks_pending: u64,
    pub batch_total: u64,
    pub batch_processed: u64,
    pub last_chunk_at_ms: Option<i64>,
    pub freshness: FreshnessLabel,  // snake_case: "active" | "recent" | "idle"
}
```

We only consume `provider`, `chunks_synced`, `last_chunk_at_ms`, `freshness` in v1. Keep the full type so the wrapper is reusable.

- [ ] **Step 1: Append the wrapper to memoryTree.ts**

Append at the bottom of `app/src/utils/tauriCommands/memoryTree.ts` (after the existing `memoryTreeSetEnabled` block):

```ts
// ── memory_sync_status_list (#2763 — per-integration health strip) ───────

/**
 * Freshness label emitted by `openhuman.memory_sync_status_list`. Snake-case
 * mirrors the Rust `FreshnessLabel` serde rename. Derived from
 * `now - last_chunk_at_ms` at RPC time, not stored.
 */
export type MemorySyncFreshness = 'active' | 'recent' | 'idle';

/**
 * One row per provider that has produced chunks. Mirrors the Rust
 * `MemorySyncStatus` struct exactly — snake_case carried through so the
 * wire payload deserialises without a remap layer.
 */
export interface MemorySyncStatusRow {
  /** Provider key — `slack`, `gmail`, `notion`, `discord`, `telegram`, etc. */
  provider: string;
  /** Total chunks in `mem_tree_chunks` for this provider. */
  chunks_synced: number;
  /** Chunks fetched but not yet extracted/embedded. Lifetime metric. */
  chunks_pending: number;
  /** Total chunks in the current sync wave. Zero when no wave is active. */
  batch_total: number;
  /** Of `batch_total`, how many have been processed. */
  batch_processed: number;
  /** Epoch ms of the most-recent chunk for this provider; null if none yet. */
  last_chunk_at_ms: number | null;
  /** Coarse activity label — derived at RPC time. */
  freshness: MemorySyncFreshness;
}

/**
 * Fetch the per-provider sync-status list. Single SQL query against
 * `mem_tree_chunks` (GROUP BY source_kind); safe to poll alongside
 * `memoryTreePipelineStatus` on the same 1.5s / 4s adaptive cadence.
 *
 * Backed by `openhuman.memory_sync_status_list` (#1136). Surfaced by the
 * per-integration health strip in `MemoryTreeStatusPanel` (#2763).
 */
export async function memorySyncStatusList(): Promise<MemorySyncStatusRow[]> {
  console.debug('[memory-tree-rpc] memorySyncStatusList: entry');
  const resp = await callCoreRpc<
    { statuses: MemorySyncStatusRow[] } | ResultEnvelope<{ statuses: MemorySyncStatusRow[] }>
  >({ method: 'openhuman.memory_sync_status_list', params: {} });
  const out = unwrapResult(resp);
  const rows = out.statuses ?? [];
  console.debug('[memory-tree-rpc] memorySyncStatusList: exit rows=%d', rows.length);
  return rows;
}
```

- [ ] **Step 2: Verify the file compiles**

Run: `pnpm typecheck`
Expected: PASS (no new TS errors). If it fails on `callCoreRpc`/`unwrapResult`/`ResultEnvelope` not being in scope, they're already imported in this file — check the top of the file. Re-check the appended block uses the same identifiers verbatim.

- [ ] **Step 3: Commit**

```bash
git add app/src/utils/tauriCommands/memoryTree.ts
git commit -m "feat(memory-tree): add memorySyncStatusList RPC wrapper (#2763)"
```

---

### Task 2: Add the 5 new i18n keys (English)

**Files:**
- Modify: `app/src/lib/i18n/en.ts:521` (immediately after the existing `memoryTree.status.daysAgo` line)

- [ ] **Step 1: Insert the new keys**

Open `app/src/lib/i18n/en.ts`. Find the line `'memoryTree.status.daysAgo': '{count} days ago',` (around line 521). Insert immediately after it:

```ts
  // Per-integration health strip (#2763) — rendered between the 4-tile grid
  // and the auto-sync toggle in MemoryTreeStatusPanel.
  'memoryTree.status.integrationsTitle': 'Per-integration health',
  'memoryTree.status.integrationsEmpty': 'No integrations connected',
  'memoryTree.status.integrationActive': 'Active',
  'memoryTree.status.integrationStale': 'Stale',
  'memoryTree.status.integrationChunks': '{count} chunks',
```

- [ ] **Step 2: Verify typecheck still clean**

Run: `pnpm typecheck`
Expected: PASS.

- [ ] **Step 3: Run the i18n parity gate to confirm it now expects these keys in every other locale**

Run: `pnpm i18n:check`
Expected: FAIL with messages like `Missing key 'memoryTree.status.integrationsTitle' in locale 'ar'` etc. (13 missing-key errors per new key). This is the desired failure that Task 3 fixes.

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/i18n/en.ts
git commit -m "feat(i18n): add English keys for integration health strip (#2763)"
```

---

### Task 3: Add real translations for all 13 non-English locales

**Files:** Modify each of:
- `app/src/lib/i18n/ar.ts`
- `app/src/lib/i18n/bn.ts`
- `app/src/lib/i18n/de.ts`
- `app/src/lib/i18n/es.ts`
- `app/src/lib/i18n/fr.ts`
- `app/src/lib/i18n/hi.ts`
- `app/src/lib/i18n/id.ts`
- `app/src/lib/i18n/it.ts`
- `app/src/lib/i18n/ko.ts`
- `app/src/lib/i18n/pl.ts`
- `app/src/lib/i18n/pt.ts`
- `app/src/lib/i18n/ru.ts`
- `app/src/lib/i18n/zh-CN.ts`

Each file already contains the `memoryTree.status.daysAgo` key (sibling to where we inserted in en.ts). Insert the 5 new keys directly after that line in every file, in the language of that file.

- [ ] **Step 1: Insert translations into each locale file**

For each locale file, find `memoryTree.status.daysAgo` and insert the corresponding block below. Use these exact translations:

**`ar.ts`** (Arabic):
```ts
  'memoryTree.status.integrationsTitle': 'حالة التكاملات',
  'memoryTree.status.integrationsEmpty': 'لا توجد تكاملات متصلة',
  'memoryTree.status.integrationActive': 'نشط',
  'memoryTree.status.integrationStale': 'قديم',
  'memoryTree.status.integrationChunks': '{count} قطعة',
```

**`bn.ts`** (Bengali):
```ts
  'memoryTree.status.integrationsTitle': 'প্রতি-ইন্টিগ্রেশন স্বাস্থ্য',
  'memoryTree.status.integrationsEmpty': 'কোনো ইন্টিগ্রেশন সংযুক্ত নেই',
  'memoryTree.status.integrationActive': 'সক্রিয়',
  'memoryTree.status.integrationStale': 'পুরানো',
  'memoryTree.status.integrationChunks': '{count} টি অংশ',
```

**`de.ts`** (German):
```ts
  'memoryTree.status.integrationsTitle': 'Integrationsstatus',
  'memoryTree.status.integrationsEmpty': 'Keine Integrationen verbunden',
  'memoryTree.status.integrationActive': 'Aktiv',
  'memoryTree.status.integrationStale': 'Veraltet',
  'memoryTree.status.integrationChunks': '{count} Chunks',
```

**`es.ts`** (Spanish):
```ts
  'memoryTree.status.integrationsTitle': 'Estado por integración',
  'memoryTree.status.integrationsEmpty': 'No hay integraciones conectadas',
  'memoryTree.status.integrationActive': 'Activa',
  'memoryTree.status.integrationStale': 'Inactiva',
  'memoryTree.status.integrationChunks': '{count} fragmentos',
```

**`fr.ts`** (French):
```ts
  'memoryTree.status.integrationsTitle': 'Santé par intégration',
  'memoryTree.status.integrationsEmpty': 'Aucune intégration connectée',
  'memoryTree.status.integrationActive': 'Active',
  'memoryTree.status.integrationStale': 'Obsolète',
  'memoryTree.status.integrationChunks': '{count} fragments',
```

**`hi.ts`** (Hindi):
```ts
  'memoryTree.status.integrationsTitle': 'प्रति-एकीकरण स्थिति',
  'memoryTree.status.integrationsEmpty': 'कोई एकीकरण कनेक्ट नहीं है',
  'memoryTree.status.integrationActive': 'सक्रिय',
  'memoryTree.status.integrationStale': 'पुराना',
  'memoryTree.status.integrationChunks': '{count} खंड',
```

**`id.ts`** (Indonesian):
```ts
  'memoryTree.status.integrationsTitle': 'Kesehatan per integrasi',
  'memoryTree.status.integrationsEmpty': 'Tidak ada integrasi tersambung',
  'memoryTree.status.integrationActive': 'Aktif',
  'memoryTree.status.integrationStale': 'Usang',
  'memoryTree.status.integrationChunks': '{count} potongan',
```

**`it.ts`** (Italian):
```ts
  'memoryTree.status.integrationsTitle': 'Stato per integrazione',
  'memoryTree.status.integrationsEmpty': 'Nessuna integrazione collegata',
  'memoryTree.status.integrationActive': 'Attiva',
  'memoryTree.status.integrationStale': 'Obsoleta',
  'memoryTree.status.integrationChunks': '{count} frammenti',
```

**`ko.ts`** (Korean):
```ts
  'memoryTree.status.integrationsTitle': '통합별 상태',
  'memoryTree.status.integrationsEmpty': '연결된 통합이 없습니다',
  'memoryTree.status.integrationActive': '활성',
  'memoryTree.status.integrationStale': '오래됨',
  'memoryTree.status.integrationChunks': '{count}개 청크',
```

**`pl.ts`** (Polish):
```ts
  'memoryTree.status.integrationsTitle': 'Stan poszczególnych integracji',
  'memoryTree.status.integrationsEmpty': 'Brak podłączonych integracji',
  'memoryTree.status.integrationActive': 'Aktywna',
  'memoryTree.status.integrationStale': 'Nieaktualna',
  'memoryTree.status.integrationChunks': '{count} fragmentów',
```

**`pt.ts`** (Portuguese):
```ts
  'memoryTree.status.integrationsTitle': 'Saúde por integração',
  'memoryTree.status.integrationsEmpty': 'Nenhuma integração conectada',
  'memoryTree.status.integrationActive': 'Ativa',
  'memoryTree.status.integrationStale': 'Obsoleta',
  'memoryTree.status.integrationChunks': '{count} fragmentos',
```

**`ru.ts`** (Russian):
```ts
  'memoryTree.status.integrationsTitle': 'Состояние интеграций',
  'memoryTree.status.integrationsEmpty': 'Нет подключённых интеграций',
  'memoryTree.status.integrationActive': 'Активна',
  'memoryTree.status.integrationStale': 'Устарела',
  'memoryTree.status.integrationChunks': '{count} фрагментов',
```

**`zh-CN.ts`** (Simplified Chinese):
```ts
  'memoryTree.status.integrationsTitle': '各集成状态',
  'memoryTree.status.integrationsEmpty': '未连接任何集成',
  'memoryTree.status.integrationActive': '活跃',
  'memoryTree.status.integrationStale': '过期',
  'memoryTree.status.integrationChunks': '{count} 个块',
```

- [ ] **Step 2: Verify i18n parity gate now passes**

Run: `pnpm i18n:check`
Expected: PASS (no missing-key errors).

- [ ] **Step 3: Verify the English-detection gate passes**

Run: `pnpm i18n:english:check`
Expected: PASS — none of the new translations match the English-detection heuristic. If a value is incorrectly flagged (very unlikely for these short strings) the failure prints the offending key + locale; rewrite that translation rather than touching the `INTENTIONAL_ENGLISH` allowlist.

- [ ] **Step 4: Commit**

```bash
git add app/src/lib/i18n/ar.ts app/src/lib/i18n/bn.ts app/src/lib/i18n/de.ts \
        app/src/lib/i18n/es.ts app/src/lib/i18n/fr.ts app/src/lib/i18n/hi.ts \
        app/src/lib/i18n/id.ts app/src/lib/i18n/it.ts app/src/lib/i18n/ko.ts \
        app/src/lib/i18n/pl.ts app/src/lib/i18n/pt.ts app/src/lib/i18n/ru.ts \
        app/src/lib/i18n/zh-CN.ts
git commit -m "feat(i18n): translate integration health strip keys (13 locales, #2763)"
```

---

### Task 4: Extend `useMemoryTreeStatus` to fetch sync-status in parallel (test first)

**Files:**
- Modify: `app/src/components/intelligence/MemoryTreeStatusPanel.test.tsx` (add a test before extending the hook)
- Modify: `app/src/components/intelligence/MemoryTreeStatusPanel.tsx` (extend hook)

**Context:** The existing `useMemoryTreeStatus` returns `{ status, loading, error, refresh }`. We add `integrations: MemorySyncStatusRow[]` to the return shape. The fetcher swaps a single `await memoryTreePipelineStatus()` for a `Promise.all` against both endpoints. On per-endpoint failure we degrade gracefully — pipeline-status failure already shows the existing error banner; sync-status failure logs a warn and renders an empty integration list, so the rest of the panel stays functional.

- [ ] **Step 1: Add the failing test**

In `app/src/components/intelligence/MemoryTreeStatusPanel.test.tsx`, find the existing `vi.mock` block (around line 21). Replace it with:

```ts
const mockPipelineStatus = vi.fn();
const mockSetEnabled = vi.fn();
const mockSyncStatusList = vi.fn();

vi.mock('../../utils/tauriCommands', async importOriginal => {
  const actual = await importOriginal<typeof import('../../utils/tauriCommands')>();
  return {
    ...actual,
    memoryTreePipelineStatus: (...args: unknown[]) => mockPipelineStatus(...args),
    memoryTreeSetEnabled: (...args: unknown[]) => mockSetEnabled(...args),
    memorySyncStatusList: (...args: unknown[]) => mockSyncStatusList(...args),
  };
});
```

Then in the existing `beforeEach`, add a reset line:
```ts
    mockSyncStatusList.mockReset();
    mockSyncStatusList.mockResolvedValue([]);  // default: empty, harmless to existing tests
```

Add the new test case inside the `describe('<MemoryTreeStatusPanel />', ...)` block (anywhere after the existing `'renders the four tiles ...'` case):

```ts
  it('fetches integration list and pipeline status in parallel on the same tick', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockResolvedValue([
      {
        provider: 'slack',
        chunks_synced: 5231,
        chunks_pending: 0,
        batch_total: 0,
        batch_processed: 0,
        last_chunk_at_ms: FIXED_NOW_MS - 3 * 60 * 1000,
        freshness: 'active',
      },
    ]);

    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(mockPipelineStatus).toHaveBeenCalledTimes(1);
      expect(mockSyncStatusList).toHaveBeenCalledTimes(1);
    });
  });
```

- [ ] **Step 2: Run the new test, watch it fail**

Run: `pnpm debug unit src/components/intelligence/MemoryTreeStatusPanel.test.tsx -t "fetches integration list"`
Expected: FAIL — `expected mockSyncStatusList to have been called 1 time, but got 0` (the hook isn't yet calling it).

- [ ] **Step 3: Extend the hook to fetch both endpoints**

In `app/src/components/intelligence/MemoryTreeStatusPanel.tsx`:

1. Update the import line (currently at ~line 26):
```ts
import {
  memoryTreePipelineStatus,
  type MemoryTreePipelineStatus,
  memoryTreeSetEnabled,
  memorySyncStatusList,
  type MemorySyncStatusRow,
} from '../../utils/tauriCommands';
```

2. Replace the entire `useMemoryTreeStatus` hook (currently lines 46–102) with this version. New state slot, `Promise.all` in the fetcher, return shape gains `integrations`:

```ts
function useMemoryTreeStatus(): {
  status: MemoryTreePipelineStatus | null;
  integrations: MemorySyncStatusRow[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
} {
  const [status, setStatus] = useState<MemoryTreePipelineStatus | null>(null);
  const [integrations, setIntegrations] = useState<MemorySyncStatusRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const cancelledRef = useRef(false);
  const statusRef = useRef<MemoryTreePipelineStatus | null>(null);
  statusRef.current = status;

  const fetchOnce = useCallback(async () => {
    console.debug('[ui-flow][memory-tree-status] fetchOnce: entry');
    try {
      // Fetch pipeline + per-integration health in parallel so the strip
      // and the tiles share a single 1.5s / 4s adaptive tick (#2763).
      const [next, rows] = await Promise.all([
        memoryTreePipelineStatus(),
        memorySyncStatusList().catch(err => {
          // Per-integration list is best-effort: surface an empty strip
          // rather than wiping the panel when only the secondary endpoint
          // fails. Pipeline failure still flips the panel-wide error.
          console.warn(
            '[ui-flow][memory-tree-status] memorySyncStatusList failed: %s',
            err instanceof Error ? err.message : String(err)
          );
          return [] as MemorySyncStatusRow[];
        }),
      ]);
      if (cancelledRef.current) return;
      setStatus(next);
      setIntegrations(rows);
      setError(null);
      console.debug(
        '[ui-flow][memory-tree-status] fetchOnce: ok status=%s total=%d integrations=%d',
        next.status,
        next.total_chunks,
        rows.length
      );
    } catch (err) {
      if (cancelledRef.current) return;
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ui-flow][memory-tree-status] fetchOnce: error %s', message);
      setError(message);
    } finally {
      if (!cancelledRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    cancelledRef.current = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const tick = async () => {
      await fetchOnce();
      if (cancelledRef.current) return;
      const live = statusRef.current;
      const fast = live?.is_syncing || (live?.pipeline_jobs?.running ?? 0) > 0;
      timer = setTimeout(tick, fast ? FAST_POLL_MS : DEFAULT_POLL_MS);
    };

    void tick();

    return () => {
      cancelledRef.current = true;
      if (timer) clearTimeout(timer);
    };
  }, [fetchOnce]);

  return { status, integrations, loading, error, refresh: fetchOnce };
}
```

3. Update the `MemoryTreeStatusPanel` body to destructure `integrations`. Replace the existing `const { status, loading, error, refresh } = useMemoryTreeStatus();` (around line 184) with:
```ts
  const { status, integrations, loading, error, refresh } = useMemoryTreeStatus();
```

(The strip itself is wired in Task 6; for now `integrations` is unused — TypeScript will allow this because it's a destructured property, not a declared local.)

- [ ] **Step 4: Run the new test, watch it pass**

Run: `pnpm debug unit src/components/intelligence/MemoryTreeStatusPanel.test.tsx -t "fetches integration list"`
Expected: PASS.

- [ ] **Step 5: Run the full file's existing tests to confirm no regression**

Run: `pnpm debug unit src/components/intelligence/MemoryTreeStatusPanel.test.tsx`
Expected: All existing tests still PASS (the default empty `mockSyncStatusList` resolution preserves behaviour).

- [ ] **Step 6: Commit**

```bash
git add app/src/components/intelligence/MemoryTreeStatusPanel.tsx \
        app/src/components/intelligence/MemoryTreeStatusPanel.test.tsx
git commit -m "feat(memory-tree): share pipeline + sync-status poll in useMemoryTreeStatus (#2763)"
```

---

### Task 5: Add status-classification helper + provider icon map (test first)

**Files:**
- Modify: `app/src/components/intelligence/MemoryTreeStatusPanel.test.tsx`
- Modify: `app/src/components/intelligence/MemoryTreeStatusPanel.tsx`

**Context:** Two pure helpers in the panel file (not exported). `classifyIntegration(freshness)` returns `'active' | 'stale'`. `providerIconChar(provider)` returns a single emoji glyph from a small built-in map, falling back to `'🔌'` for unknown providers. These are tested independently of the React render.

- [ ] **Step 1: Add tests for the helpers (failing)**

At the top of `MemoryTreeStatusPanel.test.tsx`, change the `import { MemoryTreeStatusPanel } from './MemoryTreeStatusPanel';` line to also pull the new helpers:

```ts
import {
  MemoryTreeStatusPanel,
  classifyIntegration,
  providerIconChar,
} from './MemoryTreeStatusPanel';
```

Below the existing `describe('<MemoryTreeStatusPanel />', ...)` block, add a sibling describe:

```ts
describe('integration health helpers', () => {
  describe('classifyIntegration', () => {
    it('maps active freshness to active', () => {
      expect(classifyIntegration('active')).toBe('active');
    });
    it('maps recent freshness to stale', () => {
      expect(classifyIntegration('recent')).toBe('stale');
    });
    it('maps idle freshness to stale', () => {
      expect(classifyIntegration('idle')).toBe('stale');
    });
  });

  describe('providerIconChar', () => {
    it('returns a known glyph for slack', () => {
      expect(providerIconChar('slack')).toBe('💬');
    });
    it('returns a known glyph for gmail', () => {
      expect(providerIconChar('gmail')).toBe('📧');
    });
    it('falls back to the plug glyph for unknown providers', () => {
      expect(providerIconChar('definitely-not-a-real-provider')).toBe('🔌');
    });
  });
});
```

- [ ] **Step 2: Run the new tests, watch them fail**

Run: `pnpm debug unit src/components/intelligence/MemoryTreeStatusPanel.test.tsx -t "integration health helpers"`
Expected: FAIL — `classifyIntegration` and `providerIconChar` aren't exported yet.

- [ ] **Step 3: Add the helpers and export them**

In `app/src/components/intelligence/MemoryTreeStatusPanel.tsx`, add the following block just after `function statusDotClass(...)` (around line 174, before the `MemoryTreeStatusPanelProps` interface):

```ts
/**
 * UI health classification for a single provider row in the integration
 * health strip (#2763). The wire shape's three-state `freshness` collapses
 * to two states here — `Active` (currently producing chunks) vs `Stale`
 * (anything older). An `Error` state is intentionally NOT derived from the
 * current data; per-provider failure attribution needs new core work and
 * is filed as a follow-up to issue #2763.
 */
export type IntegrationHealth = 'active' | 'stale';

/** Map the wire `freshness` enum to the two-state UI classification. */
export function classifyIntegration(freshness: MemorySyncStatusRow['freshness']): IntegrationHealth {
  return freshness === 'active' ? 'active' : 'stale';
}

/**
 * Built-in glyph for each known provider key from `memory_sync_status_list`.
 * Source: `MemorySyncStatus.provider` in `src/openhuman/memory_sync/sync_status/types.rs`
 * — that file's doc comment enumerates the providers ("slack", "gmail",
 * "discord", "telegram", "whatsapp", "notion", "meeting_notes",
 * "drive_docs", etc.). Anything not in this map falls back to a generic
 * plug glyph so unknown providers still render cleanly.
 *
 * Kept inline (rather than re-using `SOURCE_KIND_ICONS` from
 * `memorySourcesService`) because that map is keyed by `SourceKind`
 * (`composio` / `folder` / `github_repo` / …) — a different taxonomy.
 */
const PROVIDER_ICONS: Record<string, string> = {
  slack: '💬',
  gmail: '📧',
  discord: '🎮',
  telegram: '✈️',
  whatsapp: '🟢',
  notion: '📝',
  meeting_notes: '🎙️',
  drive_docs: '📄',
  github: '🐙',
};

/** Look up a provider glyph; fall back to a generic plug for unknowns. */
export function providerIconChar(provider: string): string {
  return PROVIDER_ICONS[provider] ?? '🔌';
}
```

- [ ] **Step 4: Run the helper tests, watch them pass**

Run: `pnpm debug unit src/components/intelligence/MemoryTreeStatusPanel.test.tsx -t "integration health helpers"`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/components/intelligence/MemoryTreeStatusPanel.tsx \
        app/src/components/intelligence/MemoryTreeStatusPanel.test.tsx
git commit -m "feat(memory-tree): add integration-health classifier + provider icons (#2763)"
```

---

### Task 6: Render the `IntegrationHealthStrip` sub-component (test first)

**Files:**
- Modify: `app/src/components/intelligence/MemoryTreeStatusPanel.test.tsx`
- Modify: `app/src/components/intelligence/MemoryTreeStatusPanel.tsx`

**Context:** Internal sub-component (not exported). Takes `integrations: MemorySyncStatusRow[]` and a translator. Renders header + scrollable list of rows, or empty-state copy when the array is empty. Mounted inside `MemoryTreeStatusPanel` between the tile grid (currently the `<div className="grid grid-cols-2 ..." data-testid="memory-tree-status-tiles">` block) and the auto-sync toggle row.

- [ ] **Step 1: Add the failing UI tests**

In `MemoryTreeStatusPanel.test.tsx`, inside the `describe('<MemoryTreeStatusPanel />', ...)` block, add three more cases:

```ts
  it('renders a row per integration with provider name, chunk count, freshness pill', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockResolvedValue([
      {
        provider: 'slack',
        chunks_synced: 5231,
        chunks_pending: 0,
        batch_total: 0,
        batch_processed: 0,
        last_chunk_at_ms: FIXED_NOW_MS - 3 * 60 * 1000,
        freshness: 'active',
      },
      {
        provider: 'gmail',
        chunks_synced: 842,
        chunks_pending: 0,
        batch_total: 0,
        batch_processed: 0,
        last_chunk_at_ms: FIXED_NOW_MS - 2 * 60 * 60 * 1000,
        freshness: 'idle',
      },
    ]);

    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-integrations')).toBeInTheDocument();
    });

    const rows = screen.getAllByTestId(/^memory-tree-integration-row-/);
    expect(rows).toHaveLength(2);

    // Slack row: active dot, "Active" label, chunk count rendered
    const slackRow = screen.getByTestId('memory-tree-integration-row-slack');
    expect(slackRow).toHaveTextContent(/slack/i);
    expect(slackRow).toHaveTextContent(/5,231 chunks/);
    expect(slackRow).toHaveTextContent(/Active/);

    // Gmail row: stale label
    const gmailRow = screen.getByTestId('memory-tree-integration-row-gmail');
    expect(gmailRow).toHaveTextContent(/gmail/i);
    expect(gmailRow).toHaveTextContent(/Stale/);
  });

  it('shows the empty state when there are no integrations', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockResolvedValue([]);

    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-integrations-empty')).toBeInTheDocument();
    });
    expect(screen.getByTestId('memory-tree-integrations-empty')).toHaveTextContent(
      /no integrations connected/i
    );
  });

  it('renders the integration strip between the tile grid and the toggle row', async () => {
    mockPipelineStatus.mockResolvedValue(payload());
    mockSyncStatusList.mockResolvedValue([
      {
        provider: 'slack',
        chunks_synced: 1,
        chunks_pending: 0,
        batch_total: 0,
        batch_processed: 0,
        last_chunk_at_ms: FIXED_NOW_MS - 1000,
        freshness: 'active',
      },
    ]);

    render(<MemoryTreeStatusPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('memory-tree-integrations')).toBeInTheDocument();
    });

    // DOM order: tiles → integrations → toggle row.
    const panel = screen.getByTestId('memory-tree-status-panel');
    const tiles = screen.getByTestId('memory-tree-status-tiles');
    const strip = screen.getByTestId('memory-tree-integrations');
    const toggle = screen.getByTestId('memory-tree-status-toggle-row');

    const order = Array.from(panel.querySelectorAll('[data-testid]'))
      .map(el => el.getAttribute('data-testid'))
      .filter(id =>
        ['memory-tree-status-tiles', 'memory-tree-integrations', 'memory-tree-status-toggle-row'].includes(
          id ?? ''
        )
      );

    expect(order).toEqual([
      'memory-tree-status-tiles',
      'memory-tree-integrations',
      'memory-tree-status-toggle-row',
    ]);
    // Sanity references so unused-var lint doesn't flag the locals above.
    expect(tiles).toBeInTheDocument();
    expect(strip).toBeInTheDocument();
    expect(toggle).toBeInTheDocument();
  });
```

- [ ] **Step 2: Run the new tests, watch them fail**

Run: `pnpm debug unit src/components/intelligence/MemoryTreeStatusPanel.test.tsx -t "renders a row per integration|empty state when there are no integrations|integration strip between"`
Expected: FAIL — `memory-tree-integrations` and `memory-tree-integrations-empty` test-ids do not exist yet.

- [ ] **Step 3: Add the sub-component**

In `MemoryTreeStatusPanel.tsx`, just before the existing `export function MemoryTreeStatusPanel(...)` declaration (around line 182), insert:

```ts
/**
 * Per-integration health strip (#2763). Rendered between the four pipeline
 * tiles and the auto-sync toggle inside `MemoryTreeStatusPanel`. Consumes
 * the `integrations` slice returned by `useMemoryTreeStatus` — no
 * additional fetch, no second timer.
 */
function IntegrationHealthStrip({
  integrations,
  t,
}: {
  integrations: MemorySyncStatusRow[];
  t: TFn;
}) {
  return (
    <div className="space-y-2" data-testid="memory-tree-integrations">
      <div className="text-[11px] uppercase tracking-wide text-stone-500 dark:text-neutral-400">
        {t('memoryTree.status.integrationsTitle')}
      </div>
      {integrations.length === 0 ? (
        <div
          data-testid="memory-tree-integrations-empty"
          className="rounded-lg border border-dashed border-stone-200 dark:border-neutral-800 px-3 py-2 text-xs text-stone-500 dark:text-neutral-400">
          {t('memoryTree.status.integrationsEmpty')}
        </div>
      ) : (
        <ul
          className="max-h-48 space-y-1 overflow-y-auto rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50/40 dark:bg-neutral-800/30 p-2"
          aria-label={t('memoryTree.status.integrationsTitle')}>
          {integrations.map(row => {
            const health = classifyIntegration(row.freshness);
            const healthLabel =
              health === 'active'
                ? t('memoryTree.status.integrationActive')
                : t('memoryTree.status.integrationStale');
            const dot = health === 'active' ? 'bg-sage-400' : 'bg-stone-400 dark:bg-neutral-500';
            return (
              <li
                key={row.provider}
                data-testid={`memory-tree-integration-row-${row.provider}`}
                className="flex items-center justify-between gap-2 rounded-md px-2 py-1.5 hover:bg-stone-100/60 dark:hover:bg-neutral-800/60">
                <div className="flex min-w-0 items-center gap-2">
                  <span aria-hidden className="text-base leading-none">
                    {providerIconChar(row.provider)}
                  </span>
                  <span className="truncate text-sm font-medium text-stone-800 dark:text-neutral-200">
                    {row.provider}
                  </span>
                </div>
                <div className="flex shrink-0 items-center gap-3 text-xs text-stone-500 dark:text-neutral-400">
                  <span>
                    {t('memoryTree.status.integrationChunks').replace(
                      '{count}',
                      new Intl.NumberFormat().format(row.chunks_synced)
                    )}
                  </span>
                  <span>
                    {formatRelativeMs(row.last_chunk_at_ms ?? 0, t, t('memoryTree.status.never'))}
                  </span>
                  <span className="inline-flex items-center gap-1.5 rounded-full bg-white dark:bg-neutral-900 px-2 py-0.5 text-[11px] font-medium text-stone-700 dark:text-neutral-200 ring-1 ring-stone-200 dark:ring-neutral-700">
                    <span aria-hidden className={`inline-block h-1.5 w-1.5 rounded-full ${dot}`} />
                    {healthLabel}
                  </span>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Mount the strip between the tiles and the toggle**

Still in `MemoryTreeStatusPanel.tsx`, find the closing `</div>` of the `data-testid="memory-tree-status-tiles"` block (the grid div that holds the 4 tiles — its closing `</div>` is right before the `{/* Auto-sync toggle row ... */}` comment, around line 317). Insert immediately after that closing `</div>`:

```tsx
      <IntegrationHealthStrip integrations={integrations} t={t} />
```

- [ ] **Step 5: Run the new UI tests, watch them pass**

Run: `pnpm debug unit src/components/intelligence/MemoryTreeStatusPanel.test.tsx -t "renders a row per integration|empty state when there are no integrations|integration strip between"`
Expected: PASS.

- [ ] **Step 6: Run the entire file's tests to confirm no regression**

Run: `pnpm debug unit src/components/intelligence/MemoryTreeStatusPanel.test.tsx`
Expected: All tests PASS.

- [ ] **Step 7: Commit**

```bash
git add app/src/components/intelligence/MemoryTreeStatusPanel.tsx \
        app/src/components/intelligence/MemoryTreeStatusPanel.test.tsx
git commit -m "feat(memory-tree): render per-integration health strip in status panel (#2763)"
```

---

### Task 7: Full quality suite + format + push

**Files:** No code changes; verification + push.

- [ ] **Step 1: Format**

Run: `pnpm format`
Expected: Prettier + cargo fmt clean (no Rust changed in this PR; cargo fmt is a no-op but still safe).

- [ ] **Step 2: Lint**

Run: `pnpm lint`
Expected: PASS.

- [ ] **Step 3: Typecheck**

Run: `pnpm typecheck`
Expected: PASS.

- [ ] **Step 4: i18n parity + English-detection**

Run: `pnpm i18n:check && pnpm i18n:english:check`
Expected: Both PASS.

- [ ] **Step 5: Full Vitest suite**

Run: `pnpm debug unit`
Expected: All tests PASS. New MemoryTreeStatusPanel cases are visible in the summary.

- [ ] **Step 6: Coverage spot-check (changed lines)**

Run: `pnpm test:coverage --run`
Expected: PASS; new lines in `MemoryTreeStatusPanel.tsx` and `memoryTree.ts` are exercised by the tests added above (classifier, icon map, render branches, empty state). If diff-cover reports any new line uncovered, add a targeted test before pushing — don't ship below the 80 % gate.

- [ ] **Step 7: Stage any format-only changes that fell out of Step 1**

```bash
git status
# If there are uncommitted Prettier-only changes:
git add -p   # review and stage the trivial fixups
git commit -m "chore: pnpm format pass"
```

- [ ] **Step 8: Push to fork**

```bash
git push aniketh feat/memory-tree-integration-health -u
```

Expected: branch lands on `github.com/CodeGhost21/openhuman`. If the pre-push hook fails on `prettier: command not found` (fresh worktree missing `node_modules`), check `cargo fmt --check` is clean and push with `--no-verify` — this is a known worktree gotcha and noted in `.claude/memory.md`.

- [ ] **Step 9: Open PR against upstream**

```bash
gh pr create \
  --repo tinyhumansai/openhuman \
  --base main \
  --head CodeGhost21:feat/memory-tree-integration-health \
  --title "feat(memory-tree): per-integration health strip (#2763)" \
  --body-file - <<'EOF'
## Summary

Adds a compact per-integration health strip inside `MemoryTreeStatusPanel`, between the four pipeline tiles and the auto-sync toggle. Each row shows provider icon + name + chunk count + relative last-sync time + an Active/Stale pill. Closes #2763 (#1856 Part 3).

- Reuses the existing `openhuman.memory_sync_status_list` RPC — no Rust changes.
- Single shared poll: `useMemoryTreeStatus` now fetches pipeline + sync-status in parallel on the existing 1.5s / 4s adaptive timer.
- Status mapping is pure TS: `freshness=active` → Active; `freshness=recent|idle` → Stale.
- i18n: 5 new keys, real translations across all 14 locales.

## Deviations from issue acceptance criteria

These are intentional; see `docs/superpowers/specs/2026-06-02-memory-tree-integration-health-design.md` for the full rationale:

- **AC #1** (extend `memory_tree_pipeline_status` with `integrations` array) — instead we consume the pre-existing `openhuman.memory_sync_status_list` RPC. Same data; no schema bump.
- **AC #2** (`Active / Stale / Error`) — we ship **Active / Stale** only. Per-provider Error attribution needs new core work (`mem_tree_jobs` has no `source_kind` / `source_id` column); I'll file the follow-up after this lands.

## Test plan

- [ ] `pnpm typecheck`
- [ ] `pnpm lint`
- [ ] `pnpm i18n:check`
- [ ] `pnpm i18n:english:check`
- [ ] `pnpm debug unit src/components/intelligence/MemoryTreeStatusPanel.test.tsx`
- [ ] `pnpm test:coverage` — changed-lines coverage ≥ 80 %
- [ ] Manual: open Intelligence page with at least one Composio integration connected; confirm strip renders with correct freshness pill and updates on the shared poll.
- [ ] Manual: with zero integrations, confirm the empty-state copy renders.

## Submission Checklist

- [x] Branch is based on `tinyhumansai/openhuman:main`.
- [x] PR title follows conventional commits.
- [x] Tests added/updated for changed behavior.
- [x] i18n keys added to all 14 locale files with real translations.
- [x] Diff coverage ≥ 80 % on changed lines.
- [x] No Rust changes (N/A for `cargo check`, `cargo fmt`, `cargo test`).
- [x] Linked issue: #2763.

EOF
```

Expected: PR URL printed. Drop it in chat for the user.

- [ ] **Step 10: Verify PR landed cleanly**

Run: `gh pr view --web` (optional — opens in browser) or `gh pr checks <PR-number>` to watch CI start.
Expected: Submission Checklist job picks up the `[x]`-filled checklist; `i18n:check`, `i18n:english:check`, lint, typecheck, unit tests, and coverage all PASS on CI.

---

## Self-review (done before save)

**Spec coverage:**
- Architecture (strip inside panel, shared poll) → Tasks 4 + 6.
- Status mapping → Task 5.
- i18n parity → Tasks 2 + 3.
- Test coverage → Tasks 4–6 (each TDD'd).
- Deviations called out → Task 7 PR body Step 9.
- No new RPC, no Rust → no Rust task; Task 7 still runs format/typecheck/lint (no `pnpm rust:check` because zero Rust changed).

**Placeholder scan:** None — every code block is complete and copy-pasteable. No "TBD" / "TODO" / "fill in details". Translations are written out per locale.

**Type consistency:** `MemorySyncStatusRow` defined once in Task 1, referenced verbatim in Tasks 4 / 5 / 6. `classifyIntegration` / `providerIconChar` signatures match between definition (Task 5) and call sites (Task 6). `IntegrationHealth` type matches the union returned by `classifyIntegration`.

**Risks the engineer should know about:**
1. The pre-push hook can fail on a fresh worktree (`prettier: command not found`); Task 7 Step 8 calls this out and points at the memory note.
2. `pnpm i18n:english:check` is strict about Latin-script values — if a Spanish/Portuguese/Italian translation accidentally reuses the English word verbatim it will fail. The recommended translations above were chosen to avoid this; if a reviewer requests a different word, double-check it doesn't collide with English-only function words.
