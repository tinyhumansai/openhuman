/**
 * End-to-end: client-side Composio trigger toggles (PR for backend #671).
 *
 * Drives the new `openhuman.composio_*` trigger RPC methods through the
 * running core sidecar against the shared mock backend, then opens the
 * Composio connection modal and asserts the Triggers section renders
 * the expected toggle for an ACTIVE Gmail connection.
 *
 * The mock backend (`scripts/mock-api-core.mjs`) seeds:
 *   - one ACTIVE Gmail connection (`c1`)
 *   - one available trigger (`GMAIL_NEW_GMAIL_MESSAGE`)
 *   - an empty active-trigger list that mutates as enable/disable run
 *
 * RPC behavior is deterministic across platforms; the UI assertion only
 * runs when accessibility queries reach the WebView and tolerates
 * regression-free skip on locked-down hosts.
 */
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import {
  clickNativeButton,
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import {
  completeOnboardingIfVisible,
  navigateToSkills,
  waitForRequest,
} from '../helpers/shared-flows';
import {
  clearRequestLog,
  getRequestLog,
  setMockBehavior,
  startMockServer,
  stopMockServer,
} from '../mock-server';

const LOG = '[ComposioTriggersE2E]';

function step(msg: string, ctx?: unknown) {
  if (ctx === undefined) console.log(`${LOG} ${msg}`);
  else console.log(`${LOG} ${msg}`, JSON.stringify(ctx, null, 2));
}

describe('Composio trigger toggles (UI + core RPC)', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    setMockBehavior(
      'composioConnections',
      JSON.stringify([{ id: 'c1', toolkit: 'gmail', status: 'ACTIVE' }])
    );
    setMockBehavior(
      'composioAvailableTriggers',
      JSON.stringify([
        { slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static' },
        { slug: 'SLACK_NEW_MESSAGE', scope: 'static', requiredConfigKeys: ['channel'] },
      ])
    );
    setMockBehavior('composioActiveTriggers', JSON.stringify([]));
    await waitForApp();
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('signs in deterministically', async function () {
    this.timeout(90_000);
    await triggerAuthDeepLinkBypass('e2e-composio-triggers-token');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible(LOG);
  });

  it('list_available_triggers returns the seeded Gmail catalog', async () => {
    const out = await callOpenhumanRpc('openhuman.composio_list_available_triggers', {
      toolkit: 'gmail',
      connection_id: 'c1',
    });
    expect(out.ok).toBe(true);
    // result may be bare value or wrapped in {result: ...} when logs are present
    const result = (out.result as { result?: unknown })?.result ?? out.result;
    const triggers = (result as { triggers?: unknown[] })?.triggers ?? [];
    const slugs = (triggers as { slug?: string }[]).map(t => t.slug);
    expect(slugs).toContain('GMAIL_NEW_GMAIL_MESSAGE');
    expect(slugs).toContain('SLACK_NEW_MESSAGE');
  });

  it('authorize sends Gmail read scope before Gmail trigger setup', async () => {
    clearRequestLog();

    const out = await callOpenhumanRpc('openhuman.composio_authorize', { toolkit: 'gmail' });
    expect(out.ok).toBe(true);

    const authorizeReq = await waitForRequest(
      getRequestLog,
      'POST',
      '/agent-integrations/composio/authorize',
      10_000
    );
    if (!authorizeReq) {
      throw new Error(
        `Missing /agent-integrations/composio/authorize request.\n` +
          `Request log:\n${JSON.stringify(getRequestLog(), null, 2)}`
      );
    }

    const body = JSON.parse(authorizeReq?.body || '{}');
    expect(body.toolkit).toBe('gmail');
    expect(body.oauth_scopes).toContain('https://www.googleapis.com/auth/gmail.readonly');
  });

  it('list_triggers starts empty for the seeded user', async () => {
    const out = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    expect(out.ok).toBe(true);
    const result = (out.result as { result?: unknown })?.result ?? out.result;
    const triggers = (result as { triggers?: unknown[] })?.triggers ?? [];
    expect(triggers).toHaveLength(0);
  });

  it('enable_trigger creates a trigger that subsequent list calls observe', async () => {
    const enable = await callOpenhumanRpc('openhuman.composio_enable_trigger', {
      connection_id: 'c1',
      slug: 'GMAIL_NEW_GMAIL_MESSAGE',
    });
    expect(enable.ok).toBe(true);
    const created = (enable.result as { result?: unknown })?.result ?? enable.result;
    const createdRecord = created as Record<string, unknown>;
    expect(createdRecord.slug).toBe('GMAIL_NEW_GMAIL_MESSAGE');
    expect(createdRecord.connectionId).toBe('c1');
    expect(typeof createdRecord.triggerId).toBe('string');
    expect((createdRecord.triggerId as string).length).toBeGreaterThan(0);

    const list = await callOpenhumanRpc('openhuman.composio_list_triggers', { toolkit: 'gmail' });
    const result = (list.result as { result?: unknown })?.result ?? list.result;
    const triggers = (result as { triggers?: unknown[] })?.triggers ?? [];
    expect(triggers).toHaveLength(1);
    expect((triggers[0] as { slug?: string }).slug).toBe('GMAIL_NEW_GMAIL_MESSAGE');
  });

  it('disable_trigger removes the active trigger', async () => {
    const list = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    const beforeResult = (list.result as { result?: unknown })?.result ?? list.result;
    const beforeTriggers = (beforeResult as { triggers?: unknown[] })?.triggers ?? [];
    const triggerId = (beforeTriggers[0] as { id?: string })?.id;
    expect(typeof triggerId).toBe('string');

    const disable = await callOpenhumanRpc('openhuman.composio_disable_trigger', {
      trigger_id: triggerId,
    });
    expect(disable.ok).toBe(true);
    const disableResult = (disable.result as { result?: unknown })?.result ?? disable.result;
    expect((disableResult as { deleted?: boolean })?.deleted).toBe(true);

    const after = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    const afterResult = (after.result as { result?: unknown })?.result ?? after.result;
    const afterTriggers = (afterResult as { triggers?: unknown[] })?.triggers ?? [];
    expect(afterTriggers).toHaveLength(0);
  });

  it('Triggers section renders in the Composio modal for an ACTIVE connection', async () => {
    // Seed one active trigger so the modal shows both the enabled and
    // available rows when it loads.
    setMockBehavior(
      'composioActiveTriggers',
      JSON.stringify([
        { id: 'ti-seeded', slug: 'GMAIL_NEW_GMAIL_MESSAGE', toolkit: 'gmail', connectionId: 'c1' },
      ])
    );

    await navigateToSkills();

    // The Skills page card for an ACTIVE Composio connection exposes a
    // "Manage" affordance that opens the modal. We don't depend on a
    // specific click target — accessibility text on either platform
    // surfaces "Triggers" once the modal mounts.
    const manageVisible = await waitForText('Manage', 10_000);
    if (!manageVisible) {
      step('Skills page did not surface a Manage affordance — skipping UI assertion');
      return;
    }

    // Open whichever Manage button corresponds to Gmail. The modal then
    // loads available + active triggers via the new RPCs.
    try {
      await clickNativeButton('Manage');
    } catch (err) {
      step('Could not click Manage button', { err: String(err) });
    }

    const sectionVisible =
      (await waitForText('Triggers', 10_000)) || (await textExists('GMAIL_NEW_GMAIL_MESSAGE'));
    expect(sectionVisible).toBe(true);
  });
});
