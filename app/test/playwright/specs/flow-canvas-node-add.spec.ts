import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * Adding a node to the Workflow Canvas, and the unsaved-changes guard around it.
 *
 * `NodePalette.tsx:6-11` documents two ways to add a node and says which one is
 * already covered:
 *
 *   - **click** an entry → `onAdd(entry)` — "the path the unit tests drive"
 *   - **drag** an entry onto the canvas → an `application/tinyflows-node`
 *     payload on `dataTransfer`, resolved by the canvas's `onDrop`
 *     (`EditableFlowCanvas.tsx:550,784`)
 *
 * The drag path is therefore the one with no coverage, and it is HTML5
 * drag-and-drop with a custom MIME type — **jsdom implements neither**, so a
 * component test cannot exercise it at all. `dataTransfer` does not exist there,
 * and no amount of `fireEvent.drop` reproduces the browser's payload plumbing.
 *
 * The leave guard is included because losing canvas edits is the expensive
 * failure on this surface: adding a node makes the editor dirty
 * (`FlowCanvasPage.tsx:359, badge :1132`), and navigating away must interrupt with a
 * confirm (`:1339`) offering both Stay (`:1345`) and Discard (`:1347`).
 */

const currentHash = (page: import('@playwright/test').Page) =>
  page.evaluate(() => window.location.hash);

/** tinyflows WorkflowGraph shape: `{ id, kind, name, config }` per node. */
function simpleGraph(name: string) {
  return { nodes: [{ id: 'trigger', kind: 'trigger', name, config: {} }], edges: [] };
}

/** `flows_create` answers a CLI envelope `{ result, logs }`, not the flow. */
async function createFlow(name: string): Promise<string> {
  const payload = await callCoreRpc<unknown>('openhuman.flows_create', {
    name,
    graph: simpleGraph('Manual start'),
  });
  const rec = payload as Record<string, unknown>;
  const flow = (rec && 'result' in rec ? rec.result : rec) as { id?: string };
  expect(flow?.id, `flows_create returned no id: ${JSON.stringify(payload)}`).toBeTruthy();
  return flow.id as string;
}

/**
 * Open a saved flow's canvas.
 *
 * Boots to `/home` first and lets its redirect settle before navigating:
 * `bootAuthenticatedPage` only awaits the `/home` → `/chat` redirect when the
 * caller asked for `/home`, so passing any other hash races it and silently
 * lands on chat.
 */
async function openCanvas(page: import('@playwright/test').Page, userId: string) {
  const name = `pw-canvas-${Date.now()}`;
  const flowId = await createFlow(name);

  await bootAuthenticatedPage(page, userId, '/home');
  await dismissWalkthroughIfPresent(page);

  // Set the hash and CONFIRM it stuck. `bootAuthenticatedPage` performs its own
  // `/home` -> `/chat` redirect, and on a cold first run that redirect can land
  // AFTER this assignment and steal it — which is why this failed only on the
  // first test of the file and not on the seven warm ones behind it. Re-assert
  // until the route we asked for is the route we are on.
  await expect
    .poll(
      async () => {
        const onTarget = await page.evaluate(
          id => window.location.hash.includes(`/flows/${id}`),
          flowId
        );
        if (!onTarget) {
          await page.evaluate(id => {
            window.location.hash = `/flows/${id}`;
          }, flowId);
        }
        return onTarget;
      },
      { timeout: 20_000 }
    )
    .toBe(true);

  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);

  await expect(page.getByTestId('flow-canvas-title')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('flow-canvas-title')).toHaveValue(name);

  // The insert palette is NOT visible by default. `FlowCanvasPage.tsx:1253`
  // passes `showPalette={sidePanel === 'legend'}` and `sidePanel` starts at
  // 'copilot' (:454), so the palette lives behind the "legend" tab of the
  // side-panel toggle. Worth knowing: the canvas empty state says "Add a node
  // from the palette on the left", but the palette is `absolute right-3`
  // (NodePalette.tsx:37) and hidden until this click.
  await page.getByTestId('flow-canvas-legend-toggle').click();
  await expect(page.getByTestId('flow-node-palette')).toBeVisible({ timeout: 15_000 });

  return { flowId, name };
}

/**
 * Rendered node cards. Uses xyflow's own `.react-flow__node`, which is exactly
 * one element per node — `[data-testid^="flow-node-"]` is NOT usable for a count
 * here, because `FlowNodeComponent` emits three such testids per card
 * (`flow-node-step`, `-summary`, `-validate`).
 */
const nodeCards = (page: import('@playwright/test').Page) => page.locator('.react-flow__node');

const palette = (page: import('@playwright/test').Page) => page.getByTestId('flow-node-palette');

test.describe('Flow canvas — the insert palette', () => {
  test('renders the palette alongside the saved flow', async ({ page }) => {
    await openCanvas(page, 'pw-canvas-palette');
    await expect(palette(page)).toBeVisible({ timeout: 20_000 });
    // The seeded graph has exactly one node before anything is added.
    await expect(nodeCards(page)).toHaveCount(1);
  });

  test('clicking a palette entry adds a node and marks the editor dirty', async ({ page }) => {
    await openCanvas(page, 'pw-canvas-click-add');

    // Clean before the edit: the dirty badge must not be showing.
    await expect(page.getByTestId('flow-editor-dirty')).toHaveCount(0);

    await page.getByTestId('flow-palette-item-agent').click();

    await expect(nodeCards(page)).toHaveCount(2, { timeout: 15_000 });
    await expect(page.getByTestId('flow-editor-dirty')).toBeVisible({ timeout: 15_000 });
  });

  test('dragging a palette entry onto the canvas adds a node', async ({ page }) => {
    // THE browser-only case: HTML5 drag-and-drop carrying an
    // `application/tinyflows-node` payload. jsdom has no `dataTransfer`, so no
    // component test can reach `EditableFlowCanvas`'s `onDrop` at all.
    await openCanvas(page, 'pw-canvas-drag-add');
    await expect(nodeCards(page)).toHaveCount(1);

    const source = page.getByTestId('flow-palette-item-condition');
    const target = page.locator('.react-flow__pane').first();
    await expect(target).toBeVisible({ timeout: 15_000 });

    await source.dragTo(target);

    await expect(nodeCards(page)).toHaveCount(2, { timeout: 15_000 });
    await expect(page.getByTestId('flow-editor-dirty')).toBeVisible({ timeout: 15_000 });
  });

  test('the added node is the kind that was chosen, not a default', async ({ page }) => {
    // A palette that adds the wrong kind is worse than one that adds nothing:
    // the graph looks edited and runs the wrong step.
    await openCanvas(page, 'pw-canvas-kind');

    await page.getByTestId('flow-palette-item-http_request').click();
    await expect(nodeCards(page)).toHaveCount(2, { timeout: 15_000 });

    // The palette tile carries the kind it will insert; the canvas must gain a
    // card of that same kind.
    await expect(page.locator('[data-node-kind="http_request"]').first()).toBeVisible({
      timeout: 15_000,
    });
  });
});

test.describe('Flow canvas — the unsaved-changes guard', () => {
  test('leaving with an unsaved node interrupts rather than discarding silently', async ({
    page,
  }) => {
    const { flowId } = await openCanvas(page, 'pw-canvas-leave-guard');

    await page.getByTestId('flow-palette-item-agent').click();
    await expect(page.getByTestId('flow-editor-dirty')).toBeVisible({ timeout: 15_000 });

    await page.getByTestId('flow-canvas-back').click();

    // The confirm must appear, and we must still be on the canvas.
    await expect(page.getByTestId('flow-leave-confirm')).toBeVisible({ timeout: 15_000 });
    expect(await currentHash(page)).toContain(`/flows/${flowId}`);
  });

  test('Stay keeps both the canvas and the unsaved node', async ({ page }) => {
    const { flowId } = await openCanvas(page, 'pw-canvas-leave-stay');

    await page.getByTestId('flow-palette-item-agent').click();
    await expect(nodeCards(page)).toHaveCount(2, { timeout: 15_000 });

    await page.getByTestId('flow-canvas-back').click();
    await expect(page.getByTestId('flow-leave-confirm')).toBeVisible({ timeout: 15_000 });
    await page.getByTestId('flow-leave-stay').click();

    await expect(page.getByTestId('flow-leave-confirm')).toHaveCount(0, { timeout: 15_000 });
    expect(await currentHash(page)).toContain(`/flows/${flowId}`);
    // The edit survives — Stay must not quietly roll the graph back.
    await expect(nodeCards(page)).toHaveCount(2);
    await expect(page.getByTestId('flow-editor-dirty')).toBeVisible();
  });

  test('Discard leaves the canvas and does not persist the node', async ({ page }) => {
    const { flowId } = await openCanvas(page, 'pw-canvas-leave-discard');

    await page.getByTestId('flow-palette-item-agent').click();
    await expect(nodeCards(page)).toHaveCount(2, { timeout: 15_000 });

    await page.getByTestId('flow-canvas-back').click();
    await expect(page.getByTestId('flow-leave-confirm')).toBeVisible({ timeout: 15_000 });
    await page.getByTestId('flow-leave-discard').click();

    // Poll the NARROW condition — the absence of this flow's own path. Polling
    // `toContain('/flows')` first, as an earlier draft did, is vacuous here:
    // `/flows/<id>` already contains `/flows`, so that poll returns true on its
    // first evaluation, before the click has navigated anywhere. It then left
    // the real assertion (`not.toContain('/flows/<id>')`) running unguarded
    // against an in-flight navigation — a check that was simultaneously
    // meaningless and racy. Caught in review by `tinysweeper`.
    await expect
      .poll(() => currentHash(page), { timeout: 15_000 })
      .not.toContain(`/flows/${flowId}`);

    // And the discard was real: the stored graph still has its single node.
    const payload = await callCoreRpc<unknown>('openhuman.flows_get', { id: flowId });
    const rec = payload as Record<string, unknown>;
    const flow = (rec && 'result' in rec ? rec.result : rec) as { graph?: { nodes?: unknown[] } };
    expect(flow?.graph?.nodes ?? []).toHaveLength(1);
  });

  test('a clean canvas leaves immediately, with no confirm', async ({ page }) => {
    // The guard must not nag when there is nothing to lose.
    const { flowId } = await openCanvas(page, 'pw-canvas-leave-clean');
    await expect(page.getByTestId('flow-editor-dirty')).toHaveCount(0);

    await page.getByTestId('flow-canvas-back').click();

    await expect(page.getByTestId('flow-leave-confirm')).toHaveCount(0, { timeout: 10_000 });
    // Same trap as the Discard test, and here it was the ONLY positive
    // assertion: `toContain('/flows')` is already true on `/flows/<id>`, so
    // this test reported success whether or not the back button navigated at
    // all — it could not fail. Assert that we left THIS flow's route.
    await expect
      .poll(() => currentHash(page), { timeout: 15_000 })
      .not.toContain(`/flows/${flowId}`);
  });
});
