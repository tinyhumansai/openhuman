import { fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../../services/coreRpcClient';
import { renderWithProviders } from '../../../../test/test-utils';
import DevicesPanel from '../DevicesPanel';

/**
 * `DevicesPanel` was the weakest panel on the settings surface by branch
 * coverage — 79.8% lines / 58.2% branches. The existing suite covers the device
 * list, revoke, the pair modal and the online/offline indicator; what it never
 * reaches is the formatting layer every row is rendered through, and the 2s
 * poll the pair modal turns on.
 *
 *   - `relativeTime` (panel :49-58) — five buckets, one of which every row hits.
 *   - `formatRelativeTime` (:60-76) — maps those onto i18n keys and interpolates
 *     `{count}`; a wrong bucket shows a device as last seen minutes ago when it
 *     was days.
 *   - `truncateId` (:44-47) — the boundary.
 *   - `startPolling` / `stopPolling` (:213-...) — a poll left running after the
 *     modal closes keeps hitting the core every 2s for the life of the session.
 */

vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

vi.mock('../devices/PairPhoneModal', () => ({
  default: ({ onClose, onPaired }: { onClose: () => void; onPaired: (id: string) => void }) => (
    <div data-testid="pair-modal">
      <button onClick={onClose}>close-modal</button>
      <button onClick={() => onPaired('CHAN123')}>simulate-paired</button>
    </div>
  ),
}));

const mockCall = vi.mocked(callCoreRpc);

/** A fixed clock so the relative-time buckets are deterministic. */
const NOW = new Date('2026-08-31T12:00:00.000Z').getTime();

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

function makeDevice(overrides: Record<string, unknown> = {}) {
  return {
    channel_id: 'CHAN_AAABBBCCC',
    label: "Alice's iPhone",
    device_pubkey: 'pubkey_base64url',
    created_at: new Date(NOW).toISOString(),
    last_seen_at: null,
    peer_online: false,
    revoked: false,
    ...overrides,
  };
}

/** Render a single device whose `last_seen_at` is `agoMs` in the past. */
async function renderWithLastSeen(agoMs: number | null) {
  mockCall.mockResolvedValue({
    devices: [
      makeDevice({ last_seen_at: agoMs === null ? null : new Date(NOW - agoMs).toISOString() }),
    ],
  });
  renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });
  await waitFor(() => expect(screen.getByText("Alice's iPhone")).toBeInTheDocument());
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers({ shouldAdvanceTime: true });
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe('DevicesPanel — last-seen bucketing', () => {
  it('renders a never-seen device as Never, not as 0 minutes ago', async () => {
    await renderWithLastSeen(null);
    expect(screen.getByText(/never/i)).toBeInTheDocument();
    expect(screen.queryByText(/ago/)).not.toBeInTheDocument();
  });

  it('renders a sighting under a minute as "just now"', async () => {
    await renderWithLastSeen(30_000);
    expect(screen.getByText(/just now/i)).toBeInTheDocument();
  });

  it('renders a sighting in the minutes bucket with its count', async () => {
    await renderWithLastSeen(42 * MINUTE);
    expect(screen.getByText(/42m ago/)).toBeInTheDocument();
    expect(screen.queryByText(/just now/i)).not.toBeInTheDocument();
  });

  it('renders a sighting in the hours bucket with its count', async () => {
    await renderWithLastSeen(5 * HOUR);
    expect(screen.getByText(/5h ago/)).toBeInTheDocument();
  });

  it('renders a sighting in the days bucket with its count', async () => {
    await renderWithLastSeen(3 * DAY);
    expect(screen.getByText(/3d ago/)).toBeInTheDocument();
  });

  it('does not spill a 90-minute sighting into the minutes bucket', async () => {
    // 90 minutes is 1h — reading it as "90m ago" is the classic off-by-a-bucket.
    await renderWithLastSeen(90 * MINUTE);
    expect(screen.queryByText(/90m ago/)).not.toBeInTheDocument();
    expect(screen.getByText(/1h ago/)).toBeInTheDocument();
  });

  it('does not spill a 25-hour sighting into the hours bucket', async () => {
    await renderWithLastSeen(25 * HOUR);
    expect(screen.queryByText(/25h ago/)).not.toBeInTheDocument();
    expect(screen.getByText(/1d ago/)).toBeInTheDocument();
  });

  it('treats exactly 60 minutes as the hours bucket', async () => {
    await renderWithLastSeen(60 * MINUTE);
    expect(screen.queryByText(/60m ago/)).not.toBeInTheDocument();
    expect(screen.getByText(/1h ago/)).toBeInTheDocument();
  });
});

describe('DevicesPanel — channel id truncation', () => {
  it('truncates a long channel id to first 4 + last 4', async () => {
    mockCall.mockResolvedValue({ devices: [makeDevice({ channel_id: 'ABCDEFGHIJKLMNOP' })] });
    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });

    expect(await screen.findByText('ABCD…MNOP')).toBeInTheDocument();
    expect(screen.queryByText('ABCDEFGHIJKLMNOP')).not.toBeInTheDocument();
  });

  it('leaves a 10-character id intact rather than shortening it', async () => {
    mockCall.mockResolvedValue({ devices: [makeDevice({ channel_id: 'ABCDEFGHIJ' })] });
    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });

    expect(await screen.findByText('ABCDEFGHIJ')).toBeInTheDocument();
  });
});

describe('DevicesPanel — the pair-modal poll', () => {
  async function openPairModal() {
    mockCall.mockResolvedValue({ devices: [] });
    renderWithProviders(<DevicesPanel />, { initialEntries: ['/settings/devices'] });
    await waitFor(() => expect(mockCall).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getAllByRole('button', { name: 'Pair iPhone' })[0]);
    await waitFor(() => expect(screen.getByTestId('pair-modal')).toBeInTheDocument());
  }

  it('polls the device list every 2s while the modal is open', async () => {
    await openPairModal();
    const afterOpen = mockCall.mock.calls.length;

    await vi.advanceTimersByTimeAsync(2100);
    await waitFor(() => expect(mockCall.mock.calls.length).toBeGreaterThan(afterOpen));

    const afterFirstTick = mockCall.mock.calls.length;
    await vi.advanceTimersByTimeAsync(2100);
    await waitFor(() => expect(mockCall.mock.calls.length).toBeGreaterThan(afterFirstTick));
  });

  it('stops polling once the modal closes', async () => {
    await openPairModal();
    fireEvent.click(screen.getByText('close-modal'));
    await waitFor(() => expect(screen.queryByTestId('pair-modal')).not.toBeInTheDocument());

    // Closing triggers one final reload; let it settle, then hold still.
    await waitFor(() => expect(mockCall.mock.calls.length).toBeGreaterThan(0));
    const settled = mockCall.mock.calls.length;

    await vi.advanceTimersByTimeAsync(6000);
    expect(mockCall.mock.calls.length).toBe(settled);
  });

  it('does not start a second interval when the modal is reopened', async () => {
    await openPairModal();
    fireEvent.click(screen.getByText('close-modal'));
    await waitFor(() => expect(screen.queryByTestId('pair-modal')).not.toBeInTheDocument());

    fireEvent.click(screen.getAllByRole('button', { name: 'Pair iPhone' })[0]);
    await waitFor(() => expect(screen.getByTestId('pair-modal')).toBeInTheDocument());

    const before = mockCall.mock.calls.length;
    await vi.advanceTimersByTimeAsync(2100);
    await waitFor(() => expect(mockCall.mock.calls.length).toBeGreaterThan(before));
    // One interval, so one extra call per tick — not two.
    expect(mockCall.mock.calls.length - before).toBeLessThanOrEqual(1);
  });
});
