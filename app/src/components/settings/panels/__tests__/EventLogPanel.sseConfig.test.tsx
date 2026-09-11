/**
 * EventLogPanel — the SSE config protocol.
 *
 * Before the stream sends events it may send a config frame, and that frame
 * decides two things about how the log behaves for the rest of the session:
 * `max_entries` caps the in-memory buffer, and `new_entries` ('top' | 'bottom')
 * decides whether arrivals prepend or append — which in turn decides which end
 * is trimmed when the cap is hit, and where the panel scrolls.
 *
 * The sibling spec's `mockFetchSSE` helper only ever emits plain `data:` event
 * lines, so none of that protocol was executed: `EventLogPanel.tsx` lines
 * 114-117 (the `event: config` frame) and 127-131 (the config payload) were
 * the panel's uncovered region and the reason it sat at 70.23% branch coverage.
 *
 * This is worth testing rather than assuming, because every failure mode here
 * is silent. A dropped `new_entries` makes the newest event appear at the wrong
 * end; a dropped `max_entries` lets the buffer grow past the server's cap; and
 * trimming the wrong end throws away the events the user is actually watching.
 * Nothing errors in any of those cases.
 */
import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import EventLogPanel from '../EventLogPanel';

vi.mock('../../../../services/coreRpcClient', () => ({
  getCoreHttpBaseUrl: vi.fn().mockResolvedValue('http://localhost:9999'),
  getCoreRpcToken: vi.fn().mockResolvedValue('test-token'),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

/**
 * Feed the panel raw SSE text, so a test can emit config frames as well as
 * events. The sibling helper builds `data:` lines only and cannot express this.
 */
function mockFetchRaw(body: string) {
  const encoder = new TextEncoder();
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(encoder.encode(body));
      controller.close();
    },
  });
  global.fetch = vi.fn().mockResolvedValue({ ok: true, body: stream });
}

// SSE frames are terminated by a BLANK line, not a single newline. The panel's
// parser splits on '\n' and tolerates the shorter form, but a fixture that is
// not wire-shaped would keep passing if that parser were tightened — so emit
// real frames.
const evt = (event: string, domain = 'tool') =>
  `data:${JSON.stringify({ domain, event, timestamp: '12:00:00' })}\n\n`;

const config = (payload: Record<string, unknown>) =>
  `event: config\ndata:${JSON.stringify(payload)}\n\n`;

/** Every rendered event label, in DOM order — nothing filtered out. */
function allRenderedRows(): string[] {
  return Array.from(document.querySelectorAll('span.text-xs.text-content.truncate')).map(
    el => el.textContent ?? ''
  );
}

/**
 * Rendered event labels restricted to the names a test seeded. Convenient for
 * order assertions, but blind to EXTRA rows — use `allRenderedRows` whenever
 * the point of the test is that something was not rendered.
 */
function renderedEvents(names: string[]): string[] {
  return allRenderedRows().filter(text => names.includes(text));
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('EventLogPanel SSE config frame', () => {
  it('defaults to newest-first when no config frame is sent', async () => {
    // The baseline the other cases are measured against: `newEntriesRef`
    // starts at 'top', so B (sent second) renders above A.
    mockFetchRaw(evt('AlphaEvent') + evt('BetaEvent'));
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('BetaEvent')).toBeTruthy());
    expect(renderedEvents(['AlphaEvent', 'BetaEvent'])).toEqual(['BetaEvent', 'AlphaEvent']);
  });

  it("appends newest-last when the config frame asks for new_entries 'bottom'", async () => {
    // The whole point of the frame: the server decides the direction. With
    // 'bottom' the order must invert relative to the case above.
    mockFetchRaw(
      config({ max_entries: 100, new_entries: 'bottom' }) + evt('AlphaEvent') + evt('BetaEvent')
    );
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('BetaEvent')).toBeTruthy());
    expect(renderedEvents(['AlphaEvent', 'BetaEvent'])).toEqual(['AlphaEvent', 'BetaEvent']);
  });

  it('caps the buffer at max_entries, dropping the oldest when newest-first', async () => {
    // Cap of 2 with three arrivals in 'top' mode: the list keeps the first two
    // of [C, B, A] — so C and B survive and A, the oldest, is dropped.
    mockFetchRaw(
      config({ max_entries: 2, new_entries: 'top' }) +
        evt('AlphaEvent') +
        evt('BetaEvent') +
        evt('GammaEvent')
    );
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('GammaEvent')).toBeTruthy());
    expect(renderedEvents(['AlphaEvent', 'BetaEvent', 'GammaEvent'])).toEqual([
      'GammaEvent',
      'BetaEvent',
    ]);
    expect(screen.queryByText('AlphaEvent')).toBeNull();
  });

  it('caps the buffer from the other end when newest-last', async () => {
    // Same cap, opposite direction: the list is [A, B, C] and must keep the
    // LAST two. Trimming the wrong end here would silently discard the newest
    // events instead of the oldest — which is why both directions are pinned.
    mockFetchRaw(
      config({ max_entries: 2, new_entries: 'bottom' }) +
        evt('AlphaEvent') +
        evt('BetaEvent') +
        evt('GammaEvent')
    );
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('GammaEvent')).toBeTruthy());
    expect(renderedEvents(['AlphaEvent', 'BetaEvent', 'GammaEvent'])).toEqual([
      'BetaEvent',
      'GammaEvent',
    ]);
    expect(screen.queryByText('AlphaEvent')).toBeNull();
  });

  it('ignores an out-of-range new_entries value and keeps the default', async () => {
    // The guard is `=== 'top' || === 'bottom'`. A server sending anything else
    // must leave the direction alone rather than assigning it through.
    mockFetchRaw(
      config({ max_entries: 100, new_entries: 'sideways' }) + evt('AlphaEvent') + evt('BetaEvent')
    );
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('BetaEvent')).toBeTruthy());
    expect(renderedEvents(['AlphaEvent', 'BetaEvent'])).toEqual(['BetaEvent', 'AlphaEvent']);
  });

  it('consumes the config frame instead of rendering it as an event row', async () => {
    // The frame is recognised by `max_entries !== undefined` and consumed. If
    // that check regressed, the config payload would fall through and be shown
    // as a row with no event name and domain 'unknown' — which this panel
    // renders as the uppercased domain (see the sibling spec's
    // "renders unknown domain as uppercase text").
    //
    // NOTE: this must count ALL rendered rows. An earlier draft asserted on a
    // name-filtered list and on a `badge.unknown` key that this panel never
    // emits, so it passed even with the config check disabled — the extra row
    // was simply filtered out of the comparison.
    mockFetchRaw(config({ max_entries: 50, new_entries: 'top' }) + evt('AlphaEvent'));
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('AlphaEvent')).toBeTruthy());
    expect(allRenderedRows()).toEqual(['AlphaEvent']);
    expect(screen.queryByText('UNKNOWN')).toBeNull();
  });
});
