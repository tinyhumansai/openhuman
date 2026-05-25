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
 * RPC behavior is deterministic across platforms, and the UI assertion is a
 * required part of the chain: route to Skills -> open the connected Gmail
 * modal -> verify the trigger toggles rendered.
 */
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { textExists, waitForText } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateToSkills } from '../helpers/shared-flows';
import { clearRequestLog, setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

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
    await resetApp('e2e-composio-triggers-token');
    clearRequestLog();
  });

  after(async () => {
    await stopMockServer();
  });

  it('list_available_triggers returns the seeded Gmail catalog', async () => {
    const out = await callOpenhumanRpc('openhuman.composio_list_available_triggers', {
      toolkit: 'gmail',
      connection_id: 'c1',
    });
    expect(out.ok).toBe(true);
    const result = (out.result as any)?.result ?? out.result;
    const triggers = result?.triggers ?? [];
    const slugs = triggers.map((t: any) => t.slug);
    expect(slugs).toContain('GMAIL_NEW_GMAIL_MESSAGE');
    expect(slugs).toContain('SLACK_NEW_MESSAGE');
  });

  it('list_triggers starts empty for the seeded user', async () => {
    const out = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    expect(out.ok).toBe(true);
    const result = (out.result as any)?.result ?? out.result;
    expect(result.triggers ?? []).toHaveLength(0);
  });

  it('enable_trigger creates a trigger that subsequent list calls observe', async () => {
    const enable = await callOpenhumanRpc('openhuman.composio_enable_trigger', {
      connection_id: 'c1',
      slug: 'GMAIL_NEW_GMAIL_MESSAGE',
    });
    expect(enable.ok).toBe(true);
    const created = (enable.result as any)?.result ?? enable.result;
    expect(created.slug).toBe('GMAIL_NEW_GMAIL_MESSAGE');
    expect(created.connectionId).toBe('c1');
    expect(typeof created.triggerId).toBe('string');
    expect(created.triggerId.length).toBeGreaterThan(0);

    const list = await callOpenhumanRpc('openhuman.composio_list_triggers', { toolkit: 'gmail' });
    const result = (list.result as any)?.result ?? list.result;
    expect(result.triggers).toHaveLength(1);
    expect(result.triggers[0].slug).toBe('GMAIL_NEW_GMAIL_MESSAGE');
  });

  it('disable_trigger removes the active trigger', async () => {
    const list = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    const beforeResult = (list.result as any)?.result ?? list.result;
    const triggerId = beforeResult.triggers[0]?.id;
    expect(typeof triggerId).toBe('string');

    const disable = await callOpenhumanRpc('openhuman.composio_disable_trigger', {
      trigger_id: triggerId,
    });
    expect(disable.ok).toBe(true);
    const out = (disable.result as any)?.result ?? disable.result;
    expect(out.deleted).toBe(true);

    const after = await callOpenhumanRpc('openhuman.composio_list_triggers', {});
    const afterResult = (after.result as any)?.result ?? after.result;
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

    await waitForText('Integrations', 10_000);
    await waitForText('Gmail', 10_000);

    const opened = await browser.execute(() => {
      const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>('button'));
      const gmailManage = buttons.find(button => {
        const label = button.getAttribute('aria-label') ?? '';
        return /Gmail/i.test(label) && /Manage/i.test(label);
      });
      if (!gmailManage) return false;
      ['mousedown', 'mouseup', 'click'].forEach(type => {
        gmailManage.dispatchEvent(
          new MouseEvent(type, { bubbles: true, cancelable: true, view: window, button: 0 })
        );
      });
      return true;
    });
    if (!opened) {
      throw new Error('Could not find connected Gmail Manage button on Skills page');
    }

    await waitForText('Triggers', 10_000);
    const togglesVisible = await browser.waitUntil(
      async () =>
        Boolean(
          await browser.execute(
            () => document.querySelector('[data-testid="trigger-toggles"]') !== null
          )
        ),
      { timeout: 10_000, interval: 500, timeoutMsg: 'trigger toggles did not render' }
    );
    expect(togglesVisible).toBe(true);
    expect(await textExists('Gmail New Gmail Message')).toBe(true);
  });
});
