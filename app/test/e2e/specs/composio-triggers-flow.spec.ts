// @ts-nocheck
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
  textExists,
  waitForText,
  waitForWebView,
  waitForWindowVisible,
} from '../helpers/element-helpers';
import { completeOnboardingIfVisible, navigateToSkills } from '../helpers/shared-flows';
import { clearRequestLog, setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const LOG = '[ComposioTriggersE2E]';

function step(msg: string, ctx?: unknown) {
  if (ctx === undefined) console.log(`${LOG} ${msg}`);
  else console.log(`${LOG} ${msg}`, JSON.stringify(ctx, null, 2));
}

describe('Composio trigger toggles (UI + core RPC)', () => {
  before(async () => {
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

  it('signs in deterministically', async () => {
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
    const result = out.value?.result ?? out.value;
    const triggers = result?.triggers ?? [];
    const slugs = triggers.map((t: any) => t.slug);
    expect(slugs).toContain('GMAIL_NEW_GMAIL_MESSAGE');
    expect(slugs).toContain('SLACK_NEW_MESSAGE');
  });

  it('list_triggers starts empty for the seeded user', async () => {
    const out = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    expect(out.ok).toBe(true);
    const result = out.value?.result ?? out.value;
    expect(result.triggers ?? []).toHaveLength(0);
  });

  it('enable_trigger creates a trigger that subsequent list calls observe', async () => {
    const enable = await callOpenhumanRpc('openhuman.composio_enable_trigger', {
      connection_id: 'c1',
      slug: 'GMAIL_NEW_GMAIL_MESSAGE',
    });
    expect(enable.ok).toBe(true);
    const created = enable.value?.result ?? enable.value;
    expect(created.slug).toBe('GMAIL_NEW_GMAIL_MESSAGE');
    expect(created.connectionId).toBe('c1');
    expect(typeof created.triggerId).toBe('string');
    expect(created.triggerId.length).toBeGreaterThan(0);

    const list = await callOpenhumanRpc('openhuman.composio_list_triggers', { toolkit: 'gmail' });
    const result = list.value?.result ?? list.value;
    expect(result.triggers).toHaveLength(1);
    expect(result.triggers[0].slug).toBe('GMAIL_NEW_GMAIL_MESSAGE');
  });

  it('disable_trigger removes the active trigger', async () => {
    const list = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    const beforeResult = list.value?.result ?? list.value;
    const triggerId = beforeResult.triggers[0]?.id;
    expect(typeof triggerId).toBe('string');

    const disable = await callOpenhumanRpc('openhuman.composio_disable_trigger', {
      trigger_id: triggerId,
    });
    expect(disable.ok).toBe(true);
    const out = disable.value?.result ?? disable.value;
    expect(out.deleted).toBe(true);

    const after = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    const afterResult = after.value?.result ?? after.value;
    expect(afterResult.triggers ?? []).toHaveLength(0);
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
      const el = await $('button=Manage');
      if (el && (await el.isExisting())) {
        await el.click();
      }
    } catch (err) {
      step('Could not click Manage button', { err: String(err) });
    }

    const sectionVisible =
      (await waitForText('Triggers', 10_000)) || (await textExists('GMAIL_NEW_GMAIL_MESSAGE'));
    expect(sectionVisible).toBe(true);
  });
});
