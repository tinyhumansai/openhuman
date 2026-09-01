import { expect, type Page, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * `/brain` must tell the truth about embedding state, in a real browser.
 *
 * The incident: a workspace sat with 2,581 chunks synced and 0 embedded, and no
 * degraded indicator appeared anywhere. Semantic search silently returned
 * nothing findable while every surface reported a healthy sync.
 *
 * There is already a jsdom spec proving `MemorySourceRow` *renders* the warning
 * when handed `chunks_pending > 0`
 * (`app/src/components/intelligence/MemorySourceRow.pipelineWarning.test.tsx`).
 * That proves the component. It cannot prove the thing that actually failed:
 * that a user, on the real page, against a real core, with real chunks that
 * were never embedded, SEES it. Between the component and the user sit
 * `memory_sources_status_list`, `memory_tree_pipeline_status`, the registry's
 * polling, the Brain tab routing and the row's `settled` suppression — none of
 * which jsdom exercises.
 *
 * So this spec deliberately asserts nothing about props. It seeds a folder
 * source through core RPC, syncs it for real, reads the core's own
 * `chunks_pending` to establish the incident's precondition actually holds, and
 * then asserts on rendered text.
 *
 * NOTE ON PATHS — the trap this lane is known for: a relative source path
 * resolves against the core's working directory (the build dir) and fails
 * forever with no error a user can act on. Everything here uses an absolute
 * `mkdtempSync` root, which is also what the existing
 * `intelligence-memory-ui-functional.spec.ts` does.
 */

interface SourceStatus {
  source_id: string;
  chunks_synced: number;
  chunks_pending: number;
}

async function seedDeveloperMode(page: Page): Promise<void> {
  await page.addInitScript(() => {
    try {
      const raw = localStorage.getItem('persist:theme');
      const parsed: Record<string, string> = raw ? (JSON.parse(raw) as Record<string, string>) : {};
      parsed.developerMode = JSON.stringify(true);
      localStorage.setItem('persist:theme', JSON.stringify(parsed));
    } catch {}
  });
}

async function openSources(page: Page, user: string): Promise<void> {
  await seedDeveloperMode(page);
  await bootAuthenticatedPage(page, user, '/brain?tab=sources');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
  await expect(page.getByTestId('memory-sources')).toBeVisible({ timeout: 20_000 });
}

/**
 * Every corpus this file creates, so none is left behind in the OS temp
 * directory. Each run would otherwise leak a directory of generated markdown.
 */
const createdCorpora: string[] = [];

test.afterAll(() => {
  for (const root of createdCorpora.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

/** Absolute path on purpose — see the note above. */
function makeCorpus(files: number): string {
  const root = mkdtempSync(join(tmpdir(), 'openhuman-pw-brain-'));
  createdCorpora.push(root);
  mkdirSync(join(root, 'notes'), { recursive: true });
  for (let i = 0; i < files; i += 1) {
    writeFileSync(
      join(root, 'notes', `note-${i}.md`),
      `# Note ${i}\n\nPlaywright brain canary paragraph ${i}. ${'filler '.repeat(40)}\n`
    );
  }
  return root;
}

async function addAndSync(label: string, files = 3): Promise<{ id: string; root: string }> {
  const root = makeCorpus(files);
  const added = await callCoreRpc<{ source?: { id?: string } }>('openhuman.memory_sources_add', {
    kind: 'folder',
    label,
    enabled: true,
    path: root,
    glob: '**/*.md',
  });
  const id = added?.source?.id;
  if (!id) throw new Error(`source ${label} was not created`);
  // `source_id`, not `id` — the core rejects the latter with
  // "missing required param 'source_id'". Cost the first run of this spec.
  try {
    await callCoreRpc('openhuman.memory_sources_sync', { source_id: id });
  } catch (error) {
    throw classifyModuleFailure(error);
  }
  return { id, root };
}

/**
 * The memory engine is a downloaded cdylib, and the Playwright web harness does
 * not stage it — `e2e-web-session.sh` packages and starts `openhuman-core` only,
 * unlike the Rust E2E job which installs the checksum-pinned tinymemory module.
 * With a cold module cache and no GitHub release access, `memory_sources_sync`
 * fails with `module 'tinymemory' could not be loaded` before any UI assertion
 * runs.
 *
 * That is infrastructure, not the behaviour under test, so it must not fail a
 * developer's offline run. It must equally not pass silently in CI, where a
 * missing module means this spec asserted nothing — the same void this file
 * exists to close. So: skip locally, fail loudly in CI, and say which.
 */
function classifyModuleFailure(error: unknown): Error {
  const message = error instanceof Error ? error.message : String(error);
  const moduleUnavailable = /module '[^']*' could not be loaded|github-release refused/.test(
    message
  );

  if (moduleUnavailable) {
    // CI first, and it RETURNS — so the loud path and the skip path are mutually
    // exclusive. The previous shape called `test.skip` and then fell through to
    // build the error unconditionally, which meant the caller threw it whether
    // the skip had taken effect or not, making the skip meaningless.
    if (process.env.CI) {
      return new Error(
        'the tinymemory module is not staged in this lane, so the embedding-state ' +
          'render cannot be exercised. Provision the checksum-pinned module here as ' +
          `the Rust E2E job does, or move this spec to that lane. Underlying: ${message}`
      );
    }

    // Locally this aborts the test. Nothing is built after it on this path; if
    // it ever stopped aborting, the fall-through below surfaces the ORIGINAL
    // module error rather than a misleading "not staged in this lane".
    test.skip(true, `memory module unavailable in this lane: ${message}`);
  }
  return error instanceof Error ? error : new Error(message);
}

async function statusFor(id: string): Promise<SourceStatus | undefined> {
  const res = await callCoreRpc<{ statuses: SourceStatus[] }>(
    'openhuman.memory_sources_status_list',
    {}
  );
  return res.statuses.find(s => s.source_id === id);
}

/**
 * Gate a test on the degraded precondition — loud in CI, quiet locally.
 *
 * `test.skip` alone is not safe here. If a lane has a working embeddings
 * provider (or the sync settles before the poll), `chunks_pending` is 0, all
 * three tests skip, and the file reports success having asserted nothing about
 * the render path. That is the same shape as the incident this spec exists for:
 * the incident was a correct verdict rendered into a void; a silently skipped
 * spec is a correct assertion executed into a void.
 *
 * So in CI the absence of the precondition is a FAILURE — a lane that stops
 * producing the degraded state must be fixed or the spec moved, not quietly
 * passed. Locally it still skips, because the memory engine is a downloaded
 * cdylib and a developer without release access genuinely cannot reach the
 * state.
 */
function requireDegraded(status: SourceStatus | undefined): void {
  const degraded = (status?.chunks_pending ?? 0) > 0;
  if (!degraded && process.env.CI) {
    throw new Error(
      'no unembedded chunks: this lane cannot exercise the degraded-state render. ' +
        'Stage the memory module without an embeddings provider, or move this spec ' +
        'to a lane that can.'
    );
  }
  test.skip(!degraded, 'this core embedded every chunk, so there is no degraded state to surface');
}

test.describe('Brain — the UI tells the truth about embedding state', () => {
  test('a source whose chunks were never embedded is visibly flagged, not shown as healthy', async ({
    page,
  }) => {
    const label = `PW Brain Unembedded ${Date.now()}`;
    const { id } = await addAndSync(label);

    // Establish the incident's precondition from the CORE, not from the UI.
    // If this workspace happens to have a working embeddings provider there is
    // nothing to warn about and the assertion below would be meaningless — so
    // the precondition is checked explicitly rather than assumed.
    let status: SourceStatus | undefined;
    await expect
      .poll(
        async () => {
          status = await statusFor(id);
          return status?.chunks_synced ?? 0;
        },
        { timeout: 60_000, message: 'the folder source never produced chunks' }
      )
      .toBeGreaterThan(0);

    requireDegraded(status);

    await openSources(page, 'pw-brain-unembedded');

    const row = page.getByTestId('memory-source-row-folder').filter({ hasText: label });
    await expect(row).toBeVisible({ timeout: 30_000 });

    // The whole point: rendered text a user can read, on the real page.
    await expect(row.getByTestId(`memory-source-pipeline-warning-${id}`)).toBeVisible({
      timeout: 30_000,
    });
    await expect(row).toContainText('Stored without vectors. Semantic search unavailable.');
    await expect(row).toContainText('Ingested only');
  });

  test('the warning survives a reload rather than being a first-paint artefact', async ({
    page,
  }) => {
    // A degraded state that only renders on the first poll is worse than none:
    // the user refreshes to check and the app tells them everything is fine.
    const label = `PW Brain Reload ${Date.now()}`;
    const { id } = await addAndSync(label);

    let status: SourceStatus | undefined;
    await expect
      .poll(
        async () => {
          status = await statusFor(id);
          return status?.chunks_synced ?? 0;
        },
        { timeout: 60_000 }
      )
      .toBeGreaterThan(0);
    requireDegraded(status);

    await openSources(page, 'pw-brain-reload');
    const warning = page.getByTestId(`memory-source-pipeline-warning-${id}`);
    await expect(warning).toBeVisible({ timeout: 30_000 });

    await page.reload();
    await waitForAppReady(page);
    await dismissWalkthroughIfPresent(page);

    await expect(page.getByTestId(`memory-source-pipeline-warning-${id}`)).toBeVisible({
      timeout: 30_000,
    });
  });

  test('offers a route to memory health from the flagged row', async ({ page }) => {
    // The warning is only actionable if it leads somewhere. Without this the
    // user is told semantic search is broken and given nothing to do about it.
    const label = `PW Brain Health ${Date.now()}`;
    const { id } = await addAndSync(label);

    let status: SourceStatus | undefined;
    await expect
      .poll(
        async () => {
          status = await statusFor(id);
          return status?.chunks_synced ?? 0;
        },
        { timeout: 60_000 }
      )
      .toBeGreaterThan(0);
    requireDegraded(status);

    await openSources(page, 'pw-brain-health');
    await expect(page.getByTestId(`memory-source-pipeline-warning-${id}`)).toBeVisible({
      timeout: 30_000,
    });

    // Visibility alone would pass for a disabled control or a dead handler, and
    // an unreachable escape hatch is the same as no escape hatch — the user is
    // told semantic search is broken and given a button that does nothing.
    // `onViewHealth` navigates to `/brain?tab=sync`
    // (`MemorySourcesRegistry.tsx:552-555`), so assert the destination.
    const viewHealth = page.getByTestId(`memory-source-view-health-${id}`);
    await expect(viewHealth).toBeVisible();
    await expect(viewHealth).toBeEnabled();

    await viewHealth.click();

    await expect
      .poll(async () => page.evaluate(() => window.location.hash), { timeout: 20_000 })
      .toMatch(/^#\/brain\?tab=sync/);
  });
});
