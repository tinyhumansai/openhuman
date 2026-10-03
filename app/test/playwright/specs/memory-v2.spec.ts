import { expect, type Page, type Request, type Route, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Memory v2 page (`/connections?tab=brain`, docs/specs/memory-v2.md).
 *
 * Drives the real page in a browser through its chips — Engine, Ask,
 * Learnings, Documents, Context — plus the "import previous memory" consent
 * flow and the memory-off state.
 *
 * WHY THE MEMORY RPC IS MOCKED: the page only talks to the core through the
 * `openhuman.memory_*` v2 methods (services/api/memoryApi.ts), and the engines
 * behind them are hosted (TinyHumans) or self-hosted (CortexDB) services this
 * lane must not reach. The v2 controllers may also not be in the core build the
 * lane runs against yet. So every v2 memory method is answered in-browser by a
 * small stateful fake (sources grow on add, learnings grow on learn, an import
 * goes idle -> running -> done), installed with `page.route` BEFORE the page
 * is opened. Every other method — auth, config, the app shell — passes through
 * to the real core with `route.continue()`, exactly as
 * `token-usage-load-failure.spec.ts` does, so only the memory surface is faked.
 */

const MEMORY_URL = '/#/connections?tab=brain';

const ENGINES = [
  {
    id: 'tinyhumans',
    label: 'TinyHumans',
    description: 'Hosted memory for your account.',
    hosted: true,
    needs_endpoint: false,
    needs_key: false,
    default_endpoint: null,
    fetch_modes: ['keyword', 'vector', 'hybrid'],
  },
  {
    id: 'cortexdb',
    label: 'CortexDB',
    description: 'Your own CortexDB instance.',
    hosted: false,
    needs_endpoint: true,
    needs_key: true,
    default_endpoint: 'https://api-v1.cortexdb.ai',
    fetch_modes: ['keyword', 'vector'],
  },
];

const CONTEXT_MARKDOWN =
  '# What I know about you\n\n- Prefers concise answers\n- Working on project Atlas';

interface FakeOptions {
  /** `memory_engine_get` reports TinyHumans as active (`ok`) when true, off when false. */
  engineOn: boolean;
  /** `memory_import_scan` finds v1 data to import. */
  importFound?: boolean;
}

interface RpcCall {
  method: string;
  params: Record<string, unknown>;
}

interface MemoryFake {
  /** Every intercepted memory call, in order, with the params the page sent. */
  calls: RpcCall[];
  /** Params of each call to `method` (without the `openhuman.` prefix). */
  paramsOf(method: string): Record<string, unknown>[];
}

/**
 * Install the in-browser fake for the v2 memory RPC surface. Must run before
 * the Memory page is opened so its first `memory_engine_get` is answered here.
 */
async function installMemoryFake(page: Page, opts: FakeOptions): Promise<MemoryFake> {
  const calls: RpcCall[] = [];

  const engineState = () =>
    opts.engineOn
      ? { engine: 'tinyhumans', has_key: false, status: 'ok', fetch_modes: ['keyword', 'vector'] }
      : { engine: null, has_key: false, status: 'off', fetch_modes: [] };

  const learnings: Array<Record<string, unknown>> = [
    {
      id: 'learning-seed',
      kind: 'learning',
      text: 'The user prefers dark mode.',
      meta: { source: { kind: 'agent' } },
      score: 1,
    },
  ];
  const sources: Array<Record<string, unknown>> = [];
  let nextId = 1;
  let importState = { phase: 'idle', imported: 0, total: 0, error: null as string | null };

  const context = () => ({
    markdown: CONTEXT_MARKDOWN,
    tokens: 42,
    generated_at: '2026-10-01T09:00:00Z',
    interval_mins: 360,
    budget_tokens: 1500,
    enabled: true,
  });

  const conversations = { enabled: true, batch_turns: 4, idle_secs: 120, recent: [] as unknown[] };

  /** The fake's answer per v2 method; `undefined` = not a v2 method, pass through. */
  const handle = (method: string, params: Record<string, unknown>): unknown => {
    switch (method) {
      case 'memory_engines_list':
        return { engines: ENGINES, active: opts.engineOn ? 'tinyhumans' : null };
      case 'memory_engine_get':
        return engineState();
      case 'memory_engine_set':
        return {
          engine: params.engine,
          has_key: false,
          status: 'ok',
          fetch_modes: ['keyword', 'vector'],
        };
      case 'memory_recall':
        return {
          answer: 'You decided to migrate project Atlas on Friday.',
          citations: [
            {
              id: 'cite-atlas',
              kind: 'document',
              snippet: 'Atlas migration lands Friday evening.',
              meta: { file_path: '/notes/atlas.md', source: { kind: 'folder' } },
              score: 0.92,
            },
          ],
          model: 'fake-recall',
        };
      case 'memory_fetch':
        return { hits: [], next_cursor: null };
      case 'memory_learn': {
        const id = `learning-${nextId++}`;
        learnings.unshift({
          id,
          kind: 'learning',
          text: params.text,
          meta: { source: { kind: 'agent' } },
          score: 1,
        });
        return { id };
      }
      case 'memory_forget': {
        const ids = new Set((params.ids as string[] | undefined) ?? []);
        const before = learnings.length;
        for (let i = learnings.length - 1; i >= 0; i -= 1) {
          if (ids.has(learnings[i].id as string)) learnings.splice(i, 1);
        }
        return { forgotten: before - learnings.length };
      }
      case 'memory_items_list':
        return { items: [...learnings], next_cursor: null };
      case 'memory_conversations_get':
        return conversations;
      case 'memory_conversations_set':
        Object.assign(conversations, params);
        return conversations;
      case 'memory_sources_list':
        return { sources: [...sources] };
      case 'memory_sources_add': {
        const source = {
          id: `src-${nextId++}`,
          kind: params.kind,
          target: params.target,
          label: (params.label as string | undefined) ?? params.target,
          schedule_mins: params.schedule_mins ?? null,
          last_sync_at: null,
          status: 'idle',
          error: null,
          items: 0,
        };
        sources.push(source);
        return { source };
      }
      case 'memory_sources_remove': {
        const index = sources.findIndex(s => s.id === params.id);
        if (index >= 0) sources.splice(index, 1);
        return { removed: index >= 0 };
      }
      case 'memory_sources_sync':
        return { started: params.id ? [params.id] : sources.map(s => s.id) };
      case 'memory_context_get':
      case 'memory_context_refresh':
      case 'memory_context_set':
        return context();
      case 'memory_import_scan':
        return opts.importFound
          ? { found: true, counts: { documents: 12, conversations: 3, learnings: 5 } }
          : { found: false, counts: null };
      case 'memory_import_start':
        importState = { phase: 'running', imported: 0, total: 20, error: null };
        return { state: importState };
      case 'memory_import_status': {
        const current = importState;
        // A running import finishes on the next poll, so the page walks
        // idle -> running -> done without the spec waiting on real work.
        if (importState.phase === 'running') {
          importState = { phase: 'done', imported: 20, total: 20, error: null };
        }
        return { state: current };
      }
      default:
        return undefined;
    }
  };

  await page.route('**/rpc', async (route: Route, request: Request) => {
    let body: { id?: unknown; method?: string; params?: Record<string, unknown> } = {};
    try {
      body = JSON.parse(request.postData() || '{}');
    } catch {
      await route.continue();
      return;
    }
    const full = body.method ?? '';
    if (!full.startsWith('openhuman.memory_')) {
      await route.continue();
      return;
    }
    const method = full.slice('openhuman.'.length);
    const params = body.params ?? {};
    const result = handle(method, params);
    if (result === undefined) {
      // Not part of the v2 surface: let the real core answer it.
      await route.continue();
      return;
    }
    calls.push({ method, params });
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result }),
    });
  });

  return {
    calls,
    paramsOf: (method: string) => calls.filter(c => c.method === method).map(c => c.params),
  };
}

async function openMemory(page: Page, query = '') {
  await page.goto(`${MEMORY_URL}${query}`);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('memory-page')).toBeVisible({ timeout: 30_000 });
}

const hash = (page: Page) => page.evaluate(() => window.location.hash);

test.describe('Memory v2 — engine active', () => {
  test('lands on Ask and drives engine, ask, learnings, documents and context', async ({
    page,
  }) => {
    const fake = await installMemoryFake(page, { engineOn: true });
    await bootAuthenticatedPage(page, 'pw-memory-v2-active');
    await openMemory(page);

    // 1. An active engine and no `?brain=` → the Ask chip.
    await expect(page.getByTestId('brain-tab-ask')).toHaveAttribute('aria-selected', 'true', {
      timeout: 20_000,
    });
    await expect(page.getByTestId('memory-ask-tab')).toBeVisible();

    // Engine chip lists both engines from memory_engines_list.
    await page.getByTestId('brain-tab-engine').click();
    await expect.poll(() => hash(page)).toContain('brain=engine');
    await expect(page.getByTestId('memory-engines')).toBeVisible();
    await expect(page.getByTestId('memory-engine-tinyhumans')).toBeVisible();
    await expect(page.getByTestId('memory-engine-cortexdb')).toBeVisible();
    await expect(page.getByTestId('memory-engine-tinyhumans-active')).toBeVisible();

    // 2. Ask: the recall answer and its citation render.
    await page.getByTestId('brain-tab-ask').click();
    await page.getByTestId('memory-ask-input').fill('When does Atlas migrate?');
    await page.getByTestId('memory-ask-submit').click();
    const answer = page.getByTestId('memory-ask-answer');
    await expect(answer).toContainText('migrate project Atlas on Friday');
    await expect(page.getByTestId('memory-citation-cite-atlas')).toContainText(
      'Atlas migration lands Friday evening.'
    );
    expect(fake.paramsOf('memory_recall')).toEqual([{ question: 'When does Atlas migrate?' }]);

    // 3. Learnings: a new learning is stored and listed.
    await page.getByTestId('brain-tab-learnings').click();
    await expect(page.getByTestId('memory-learning-learning-seed')).toBeVisible();
    await page.getByTestId('memory-learning-input').fill('Standups are at 9:30.');
    await page.getByTestId('memory-learning-add').click();
    const learned = page.getByTestId('memory-learning-learning-1');
    await expect(learned).toBeVisible();
    await expect(learned).toContainText('Standups are at 9:30.');
    // The default kind is `fact`; nothing else rides along.
    expect(fake.paramsOf('memory_learn')).toEqual([
      { text: 'Standups are at 9:30.', kind: 'fact' },
    ]);

    // 4. Documents: register a folder source (folder is the default kind).
    await page.getByTestId('brain-tab-documents').click();
    await expect(page.getByTestId('memory-sources-empty')).toBeVisible();
    await page.getByTestId('memory-sources-add').click();
    await expect(page.getByTestId('memory-add-source')).toBeVisible();
    await page.getByTestId('memory-add-source-target').fill('/Users/e2e/notes');
    await page.getByTestId('memory-add-source-submit').click();
    await expect(page.getByTestId('memory-add-source')).toBeHidden();
    const row = page.getByTestId('memory-source-src-2');
    await expect(row).toBeVisible();
    await expect(row).toContainText('/Users/e2e/notes');
    expect(fake.paramsOf('memory_sources_add')).toEqual([
      { kind: 'folder', target: '/Users/e2e/notes' },
    ]);

    // 5. Context: the compiled context.md renders as markdown.
    await page.getByTestId('brain-tab-context').click();
    const markdown = page.getByTestId('memory-context-markdown');
    await expect(markdown).toContainText('What I know about you');
    await expect(markdown).toContainText('Prefers concise answers');
  });

  test('importing previous memory needs explicit consent', async ({ page }) => {
    const fake = await installMemoryFake(page, { engineOn: true, importFound: true });
    await bootAuthenticatedPage(page, 'pw-memory-v2-import');
    await openMemory(page, '&brain=ask');

    // 6. The scan found v1 data: the banner offers it with the counts.
    const banner = page.getByTestId('memory-import-banner');
    await expect(banner).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('memory-import-counts')).toContainText('12 documents');

    // Opening the offer only asks; nothing is uploaded yet.
    await page.getByTestId('memory-import-open').click();
    const consent = page.getByTestId('memory-import-consent');
    await expect(consent).toBeVisible();
    await expect(consent).toContainText('TinyHumans');
    expect(fake.paramsOf('memory_import_start')).toEqual([]);

    await page.getByTestId('memory-import-confirm').click();
    await expect(consent).toBeHidden();
    expect(fake.paramsOf('memory_import_start')).toEqual([{ consent: true }]);

    // Running first, then done once the status poll sees it finish.
    await expect(
      page.getByTestId('memory-import-running').or(page.getByTestId('memory-import-done'))
    ).toBeVisible();
    await expect(page.getByTestId('memory-import-done')).toBeVisible({ timeout: 15_000 });
  });

  test('legacy Brain and settings links land on their v2 chips', async ({ page }) => {
    await installMemoryFake(page, { engineOn: true });
    await bootAuthenticatedPage(page, 'pw-memory-v2-legacy');

    const cases: Array<[string, RegExp]> = [
      ['/#/brain?tab=graph', /^#\/connections\?tab=brain&brain=ask(?:&|$)/],
      ['/#/brain?tab=sources', /^#\/connections\?tab=brain&brain=documents(?:&|$)/],
      [`${MEMORY_URL}&brain=sync`, /^#\/connections\?tab=brain&brain=documents(?:&|$)/],
      ['/#/settings/memory-engine', /^#\/connections\?tab=brain&brain=engine(?:&|$)/],
      ['/#/settings/memory-data', /^#\/connections\?tab=brain&brain=documents(?:&|$)/],
      ['/#/settings/memory-debug', /^#\/connections\?tab=brain&brain=ask(?:&|$)/],
    ];
    for (const [from, to] of cases) {
      await page.goto(from);
      await waitForAppReady(page);
      await expect.poll(() => hash(page), { message: `${from} should redirect` }).toMatch(to);
      await expect(page.getByTestId('memory-page')).toBeVisible();
    }
  });
});

test.describe('Memory v2 — memory off', () => {
  test('opens on Engine and every other chip shows the off state', async ({ page }) => {
    const fake = await installMemoryFake(page, { engineOn: false });
    await bootAuthenticatedPage(page, 'pw-memory-v2-off');
    await openMemory(page);

    // No usable engine and no `?brain=` → the Engine chip, with the off banner.
    await expect(page.getByTestId('brain-tab-engine')).toHaveAttribute('aria-selected', 'true', {
      timeout: 20_000,
    });
    await expect(page.getByTestId('memory-engine-status-off')).toBeVisible();
    await expect(page.getByTestId('memory-engines')).toBeVisible();

    // A content chip explains memory is off and points back to Engine.
    await page.goto(`${MEMORY_URL}&brain=documents`);
    await expect(page.getByTestId('memory-off-state')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('memory-documents-tab')).toHaveCount(0);
    await page.getByTestId('memory-off-open-engine').click();
    await expect.poll(() => hash(page)).toContain('brain=engine');
    await expect(page.getByTestId('memory-engine-tab')).toBeVisible();

    // With memory off the page never offers an import or lists sources.
    expect(fake.paramsOf('memory_import_scan')).toEqual([]);
    expect(fake.paramsOf('memory_sources_list')).toEqual([]);
  });
});
