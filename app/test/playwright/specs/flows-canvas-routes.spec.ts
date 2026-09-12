import { expect, test } from '@playwright/test';

import {
  bootAuthenticatedPage,
  callCoreRpc,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

/**
 * The Flows routes, driven in a real browser: the list, an existing flow's
 * canvas, the unsaved-draft route, and the two back-compat slugs.
 *
 * Existing coverage is thin and indirect — `skill-lifecycle.spec.ts` only
 * checks that the `flows_list` RPC is reachable, and
 * `top-level-functional-flows.spec.ts` drives the *legacy* SKILL.md
 * `/workflows` CRUD page, which is a different surface from the `flows::`
 * canvas. Nothing opens `/flows/:id`.
 *
 * A note on `/flows/draft`, because the brief expected it to open a canvas and
 * it deliberately does not: the draft graph rides in `location.state`
 * (`FlowCanvasPage.tsx:1546-1558`), so a direct URL hit or a hard reload has no
 * draft and the route renders an empty state on purpose — "rather than a broken
 * canvas". That is the behaviour pinned below. Reaching the draft canvas needs
 * the chat WorkflowProposalCard's "Open in canvas" action, which is a chat-side
 * entry point outside this brief's routes.
 */

const currentHash = (page: import('@playwright/test').Page) =>
  page.evaluate(() => window.location.hash);

/**
 * `flows_create` answers a CLI envelope — `{ result, logs }` — not the flow
 * directly; `flowsApi.createFlow` unwraps it with its own `unwrapCliEnvelope`.
 * Reading `.id` off the raw RPC result yields undefined while the core happily
 * logs `flows_create -> ok`, which is a confusing pair to debug.
 */
async function createFlow(name: string): Promise<string> {
  const payload = await callCoreRpc<unknown>('openhuman.flows_create', {
    name,
    graph: simpleGraph('Manual start'),
  });
  const rec = payload as Record<string, unknown>;
  const flow = (rec && 'result' in rec ? rec.result : rec) as { id?: string };
  const id = flow?.id;
  expect(id, `flows_create returned no id (payload: ${JSON.stringify(payload)})`).toBeTruthy();
  return id as string;
}

/**
 * The create affordance. On an empty account `/flows` renders a marketing empty
 * state whose button carries no testid; once flows exist the list header's
 * `flows-new-workflow` appears. Match on the accessible name so the assertion
 * holds in both states — which is what a user actually sees either way.
 */
const newWorkflowButton = (page: import('@playwright/test').Page) =>
  page.getByRole('button', { name: /new workflow/i }).first();

/**
 * A minimal graph the core accepts. tinyflows nodes are
 * `{ id, kind, name, config }` — `kind` is the discriminator (`NodeKindContract`
 * in `vendor/tinyflows`), NOT the ReactFlow-shaped `{type, position, data}` the
 * canvas components use. Passing the canvas shape here fails validation with
 * `missing field 'kind'`.
 */
function simpleGraph(name: string) {
  return { nodes: [{ id: 'trigger', kind: 'trigger', name, config: {} }], edges: [] };
}

async function goto(page: import('@playwright/test').Page, route: string) {
  await page.evaluate(target => {
    window.location.hash = target;
  }, route);
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);
}

test.describe('Flows — the list page', () => {
  test('renders the list surface with its create affordance', async ({ page }) => {
    // Boot to /home and let its redirect to /chat settle FIRST. Passing '/flows'
    // straight to bootAuthenticatedPage races that redirect, which lands the
    // page back on chat — the failure looks like a missing testid.
    await bootAuthenticatedPage(page, 'pw-flows-list', '/home');
    await dismissWalkthroughIfPresent(page);
    await goto(page, '/flows');

    await expect(newWorkflowButton(page)).toBeVisible({ timeout: 20_000 });
    // And the page is the Flows surface, not some other page that happens to
    // have a similarly named button.
    expect(await currentHash(page)).toContain('/flows');
  });
});

test.describe('Flows — an existing flow opens its canvas', () => {
  test('/flows/:id loads the saved flow and shows its title', async ({ page }) => {
    const name = `pw-flow-${Date.now()}`;
    const flowId = await createFlow(name);

    await bootAuthenticatedPage(page, 'pw-flows-canvas', '/home');
    await dismissWalkthroughIfPresent(page);
    await goto(page, `/flows/${flowId}`);

    // The canvas header carries the flow name; `flow-canvas-not-found` is the
    // negative case and must NOT be what renders.
    await expect(page.getByTestId('flow-canvas-title')).toBeVisible({ timeout: 20_000 });
    // `flow-canvas-title` is an <input> (the rename field, FlowCanvasPage:1178),
    // so the name is its VALUE. `toContainText` reads text content and would
    // always see "" here — a green-looking assertion that checks nothing.
    await expect(page.getByTestId('flow-canvas-title')).toHaveValue(name);
    await expect(page.getByTestId('flow-canvas-not-found')).toHaveCount(0);
  });

  test('an unknown flow id shows the not-found state, not a blank canvas', async ({ page }) => {
    await bootAuthenticatedPage(page, 'pw-flows-404', '/home');
    await dismissWalkthroughIfPresent(page);
    await goto(page, '/flows/pw-definitely-not-a-real-flow-id');

    await expect(
      page.getByTestId('flow-canvas-not-found').or(page.getByTestId('flow-canvas-error')).first()
    ).toBeVisible({ timeout: 20_000 });
    // A back affordance must exist so the user is not stranded.
    await expect(page.getByTestId('flow-canvas-back')).toBeVisible();
  });

  test('the canvas back button returns to the list', async ({ page }) => {
    const name = `pw-flow-back-${Date.now()}`;
    const flowId = await createFlow(name);

    await bootAuthenticatedPage(page, 'pw-flows-back', '/home');
    await dismissWalkthroughIfPresent(page);
    await goto(page, `/flows/${flowId}`);
    await expect(page.getByTestId('flow-canvas-title')).toBeVisible({ timeout: 20_000 });

    await page.getByTestId('flow-canvas-back').click();
    // Not `toContain('/flows')` — `/flows/<id>` already satisfies that, so the
    // poll would pass before the click navigated. Nobody flagged this one; it
    // is the same defect `tinysweeper` found twice in the node-add spec, and
    // the only reason it was not load-bearing here is the list-only assertion
    // below. Fixed anyway, so the poll means what it appears to mean.
    await expect
      .poll(() => currentHash(page), { timeout: 15_000 })
      .not.toContain(`/flows/${flowId}`);
    await expect(newWorkflowButton(page)).toBeVisible({ timeout: 20_000 });
  });
});

test.describe('Flows — the unsaved draft route', () => {
  test('/flows/draft hit directly shows an empty state, not a broken canvas', async ({ page }) => {
    // Pinning documented behaviour: the draft graph lives in `location.state`,
    // so a direct hit or a reload legitimately has nothing to render
    // (FlowCanvasPage.tsx:1550-1558).
    await bootAuthenticatedPage(page, 'pw-flows-draft', '/home');
    await dismissWalkthroughIfPresent(page);
    await goto(page, '/flows/draft');

    // Assert the draft empty state BY NAME. An earlier draft asserted only
    // "back button visible, no title", which the unknown-flow screen also
    // satisfies — it has a back affordance and no title either — so the test
    // passed on the wrong screen. `flow-canvas-draft-missing`
    // (FlowCanvasPage.tsx:1595) is unique to this state. Caught in review by
    // `coderabbitai`.
    await expect(page.getByTestId('flow-canvas-draft-missing')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('flow-canvas-back')).toBeVisible();
    await expect(page.getByTestId('flow-canvas-title')).toHaveCount(0);
  });

  test('"draft" is not treated as a flow id', async ({ page }) => {
    // The route order matters: `/flows/draft` is declared before `/flows/:id`
    // (AppRoutes.tsx:119-131) precisely so `:id` cannot capture "draft" and
    // fire `flows_get('draft')`. If that order regressed, the not-found state
    // would render instead of the draft empty state.
    await bootAuthenticatedPage(page, 'pw-flows-draft-order', '/home');
    await dismissWalkthroughIfPresent(page);
    await goto(page, '/flows/draft');

    // Positively the draft state, not merely "not the not-found state": the
    // previous form excluded `flow-canvas-not-found` but still accepted
    // `flow-canvas-error`, so a `flows_get('draft')` that failed loudly would
    // have passed this test.
    await expect(page.getByTestId('flow-canvas-draft-missing')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('flow-canvas-not-found')).toHaveCount(0);
    await expect(page.getByTestId('flow-canvas-error')).toHaveCount(0);
  });
});

test.describe('Flows — the back-compat slugs resolve distinctly', () => {
  // These three are NOT interchangeable, and an earlier draft of this file
  // asserted only "body is non-empty and some button is visible" — which the
  // chat surface satisfies, so it passed without ever reaching the right page.
  // Each now asserts the specific surface it must land on.

  test('/routines redirects to the Flows list', async ({ page }) => {
    // AppRoutes.tsx:215 — <Navigate to="/flows" replace />.
    await bootAuthenticatedPage(page, 'pw-slug-routines', '/home');
    await dismissWalkthroughIfPresent(page);
    await goto(page, '/routines');

    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('/flows');
    await expect(newWorkflowButton(page)).toBeVisible({ timeout: 20_000 });
  });

  test('/workflows keeps the legacy hub and does NOT redirect to Flows', async ({ page }) => {
    // AppRoutes.tsx:229 renders <Activity />, deliberately: the comment there
    // says "Keep the legacy top-level hub reachable". Conflating it with /flows
    // is the mistake this test exists to catch.
    await bootAuthenticatedPage(page, 'pw-slug-workflows', '/home');
    await dismissWalkthroughIfPresent(page);
    await goto(page, '/workflows');

    expect(await currentHash(page)).toContain('/workflows');
    expect(await currentHash(page)).not.toContain('/flows');
    // Its own create affordance, not the Flows one.
    await expect(page.getByTestId('workflows-create-btn')).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId('flows-new-workflow')).toHaveCount(0);
  });

  test('/webhooks resolves through two hops and lands on Connections', async ({ page }) => {
    // Two redirects, not one: AppRoutes.tsx:238 sends /webhooks to
    // /settings/integrations, and settingsRouteElements.tsx:129 sends THAT to
    // /connections. The intermediate target no longer exists as a page.
    //
    // Worth knowing when reading the code: the repo docs describe this hop as
    // landing on `/settings/integrations#webhooks`, but no `#webhooks` fragment
    // survives either hop, so an old webhooks bookmark arrives at the generic
    // Connections page with nothing pointing at where webhooks went. Recorded
    // in W3-ui-bugs.md; this test pins the CURRENT destination.
    await bootAuthenticatedPage(page, 'pw-slug-webhooks', '/home');
    await dismissWalkthroughIfPresent(page);
    await goto(page, '/webhooks');

    await expect.poll(() => currentHash(page), { timeout: 15_000 }).toContain('/connections');
    // And it does not strand the user mid-hop on a route with no page.
    expect(await currentHash(page)).not.toContain('/settings/integrations');
  });
});
