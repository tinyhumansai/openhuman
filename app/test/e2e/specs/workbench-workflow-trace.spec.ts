// @ts-nocheck
/**
 * M1.3.2 / youpet-core#15 — committed-code operator acceptance.
 *
 * The companion root harness starts a disposable PostgreSQL cluster and a
 * live YouPet Core. This spec then drives the built Tauri/CEF application,
 * so every assertion crosses the production React -> Tauri RPC -> Rust HTTP
 * bridge instead of replacing that boundary with a browser mock.
 */
import { waitForApp } from '../helpers/app-helpers';
import { captureCheckpoint } from '../helpers/artifacts';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-m132-workflow-trace';
const TASK_ALERT_SUMMARY = 'Owner missed two check-ins.';
const PARTIAL_ALERT_SUMMARY = 'Unsupported workflow anchor.';

async function waitForArticle(summary: string, timeout = 20_000): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute((targetSummary: string) => {
        return Array.from(document.querySelectorAll('article')).some(article =>
          (article.textContent ?? '').includes(targetSummary)
        );
      }, summary),
    { timeout, interval: 250, timeoutMsg: `workbench article "${summary}" did not appear` }
  );
}

async function openTraceForArticle(summary: string): Promise<void> {
  await waitForArticle(summary);
  const opened = await browser.execute(target => {
    const article = Array.from(document.querySelectorAll('article')).find(candidate =>
      (candidate.textContent ?? '').includes(target)
    );
    const traceButton = Array.from(article?.querySelectorAll('button') ?? []).find(
      button => button.textContent?.trim() === 'Trace'
    );
    traceButton?.click();
    return Boolean(traceButton);
  }, summary);
  expect(opened).toBe(true);

  await browser.waitUntil(
    async () =>
      browser.execute(() => {
        const dialog = document.querySelector('[role="dialog"]');
        return Boolean(dialog && !(dialog.textContent ?? '').includes('Loading trace'));
      }),
    {
      timeout: 20_000,
      interval: 250,
      timeoutMsg: `trace drawer for "${summary}" did not finish loading`,
    }
  );
}

async function closeTrace(): Promise<void> {
  const closed = await browser.execute(() => {
    const dialog = document.querySelector('[role="dialog"]');
    const closeButton = Array.from(dialog?.querySelectorAll('button') ?? []).find(
      button => button.textContent?.trim() === 'Close'
    );
    closeButton?.click();
    return Boolean(closeButton);
  });
  expect(closed).toBe(true);
  await browser.$('[role="dialog"]').waitForExist({ timeout: 5_000, reverse: true });
}

describe('M1.3.2 workflow trace operator acceptance', function () {
  this.timeout(120_000);

  before(async function beforeSuite() {
    await startMockServer(Number(process.env.E2E_MOCK_PORT || 18473));
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('shows workflow failure, retry, recovery, provenance, and bounded metadata read-only', async function () {
    this.timeout(120_000);
    await navigateViaHash('/workbench');
    await waitForArticle(TASK_ALERT_SUMMARY);

    await openTraceForArticle(TASK_ALERT_SUMMARY);
    const traceState = await browser.execute(() => {
      const metadataFromEntry = (entry: Element): Record<string, string> => {
        const root = entry.querySelector('[aria-label="Metadata"]');
        const pairs: Record<string, string> = {};
        if (!root) return pairs;
        for (const chip of Array.from(root.querySelectorAll(':scope > span'))) {
          const chipText = (chip.textContent ?? '').trim();
          const separator = chipText.indexOf(':');
          if (separator < 0) continue;
          pairs[chipText.slice(0, separator).trim()] = chipText.slice(separator + 1).trim();
        }
        return pairs;
      };
      const fieldsFromEntry = (entry: Element): Record<string, string> => {
        const fields: Record<string, string> = {};
        for (const row of Array.from(entry.querySelectorAll('dl > div'))) {
          const term = (row.querySelector('dt')?.textContent ?? '').trim();
          const definition = (row.querySelector('dd')?.textContent ?? '').trim();
          if (term) fields[term] = definition;
        }
        return fields;
      };
      const dialog = document.querySelector('[role="dialog"]');
      const text = dialog?.textContent ?? '';
      const buttons = Array.from(dialog?.querySelectorAll('button') ?? []).map(button =>
        button.textContent?.trim()
      );
      const entries = Array.from(dialog?.querySelectorAll('ol li') ?? []).map(entry => ({
        text: entry.textContent ?? '',
        fields: fieldsFromEntry(entry),
        metadata: metadataFromEntry(entry),
      }));
      return { text, buttons, entries };
    });

    expect(traceState.text).toContain('Workflow summary');
    for (const lane of ['Step', 'Event', 'Delivery', 'Audit']) {
      expect(traceState.text).toContain(lane);
    }
    const failureIndex = traceState.entries.findIndex(entry =>
      entry.text.includes('Failed · Retry scheduled')
    );
    const recoveryIndex = traceState.entries.findIndex(
      entry => entry.text.includes('Delivery recovered') || entry.text.includes('Recovered')
    );
    expect(failureIndex).toBeGreaterThanOrEqual(0);
    expect(recoveryIndex).toBeGreaterThan(failureIndex);

    const failureEntry = traceState.entries[failureIndex];
    const recoveryEntry = traceState.entries[recoveryIndex];
    expect(failureEntry.fields.Actor).toBe('Agent · openclaw-youpet-consumer');
    expect(failureEntry.fields.Related).toBe('event_outbox / 00000000-0000-0000-0000-000000000801');
    expect(failureEntry.metadata.consumer).toBe('openclaw');
    expect(failureEntry.metadata.attempts).toBe('1');
    expect(failureEntry.metadata.action).toBe('outbox.nack');
    expect(recoveryEntry.fields.Actor).toBe('Agent · openclaw-youpet-consumer');
    expect(recoveryEntry.fields.Related).toBe(
      'event_outbox / 00000000-0000-0000-0000-000000000801'
    );
    expect(recoveryEntry.metadata.consumer).toBe('openclaw');
    expect(recoveryEntry.metadata.attempts).toBe('1');
    expect(recoveryEntry.metadata.action).toBe('outbox.ack');
    expect(traceState.text).toContain('correlation_id');
    expect(traceState.text).toContain('corr_seed');

    expect(traceState.buttons).toEqual(['Close', 'Refresh trace']);
    expect(traceState.text).not.toContain('dev-openhuman-token');
    expect(traceState.text).not.toContain('raw_secret');
    expect(traceState.text).not.toContain('service_token');
    for (const mutation of ['Retry', 'Redrive', 'Approve', 'Reject']) {
      expect(traceState.buttons).not.toContain(mutation);
    }

    await captureCheckpoint('m132-workflow-summary');
    const scrolledToFailure = await browser.execute(() => {
      const failureEntry = Array.from(document.querySelectorAll('li')).find(entry =>
        (entry.textContent ?? '').includes('Failed · Retry scheduled')
      );
      failureEntry?.scrollIntoView({ block: 'center' });
      return Boolean(failureEntry);
    });
    expect(scrolledToFailure).toBe(true);
    await browser.pause(250);
    await captureCheckpoint('m132-workflow-failure-recovery');
    await closeTrace();
  });

  it('surfaces an unsupported anchor as an explicit partial trace warning', async function () {
    this.timeout(90_000);
    await openTraceForArticle(PARTIAL_ALERT_SUMMARY);

    const partialState = await browser.execute(() => {
      const dialog = document.querySelector('[role="dialog"]');
      return {
        text: dialog?.textContent ?? '',
        buttons: Array.from(dialog?.querySelectorAll('button') ?? []).map(button =>
          button.textContent?.trim()
        ),
        entries: Array.from(dialog?.querySelectorAll('ol li') ?? []).map(
          entry => entry.textContent ?? ''
        ),
      };
    });

    expect(partialState.text).toContain('Workflow identity is unavailable for this alert.');
    expect(partialState.text).toContain('Unsupported Related Type');
    expect(partialState.text).toContain(
      'trace projection does not support related_type operator_fixture'
    );
    expect(partialState.entries.some(entry => entry.includes('Alert created'))).toBe(true);
    expect(partialState.entries.some(entry => entry.includes('Unsupported workflow anchor.'))).toBe(
      true
    );
    expect(partialState.buttons).toEqual(['Close', 'Refresh trace']);
    await captureCheckpoint('m132-explicit-partial-warning');
  });
});
