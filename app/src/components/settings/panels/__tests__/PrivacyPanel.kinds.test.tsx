/**
 * PrivacyPanel — the data-kind labels, the empty catalog, and the analytics
 * toggle's failure path.
 *
 * `PrivacyPanel.test.tsx` (sibling) exercises `raw` and `derived` only, so
 * three of the five `kindLabel` arms (`PrivacyPanel.tsx:36-49`) never ran, and
 * neither did the "ready but empty" state nor the toggle's `catch`.
 *
 * The labels are the point of the panel: it is the screen that tells a user
 * what leaves their computer, so a capability rendering under the wrong kind —
 * `credentials` shown as `raw`, say — misinforms them about exactly the thing
 * they came here to check.
 *
 * Separate file rather than an edit to the sibling: three of us are in this
 * directory.
 */
import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { type Capability, listCapabilities } from '../../../../utils/tauriCommands/aboutApp';
import PrivacyPanel from '../PrivacyPanel';

vi.mock('../../../../utils/tauriCommands/aboutApp', () => ({ listCapabilities: vi.fn() }));

const setAnalyticsEnabledMock = vi.fn();
vi.mock('../../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    snapshot: { analyticsEnabled: false },
    setAnalyticsEnabled: (v: boolean) => setAnalyticsEnabledMock(v),
  }),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

const mockList = vi.mocked(listCapabilities);

type DataKind = 'raw' | 'derived' | 'credentials' | 'diagnostics' | 'metadata';

const cap = (id: string, data_kind: DataKind, over: Partial<Capability> = {}): Capability => ({
  id,
  name: `Cap ${id}`,
  domain: 'settings',
  category: 'settings',
  description: `Description for ${id}`,
  how_to: 'Somewhere',
  status: 'stable',
  privacy: { leaves_device: true, data_kind, destinations: ['Backend'] },
  ...over,
});

const row = (id: string) => screen.getByTestId(`privacy-row-${id}`);

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

afterEach(() => vi.restoreAllMocks());

describe('PrivacyPanel — data-kind labels', () => {
  // Every arm of `kindLabel`. The switch has no default, so a new kind added to
  // the union without a case here returns undefined and the badge renders
  // blank — this is what would catch that.
  it.each([
    ['raw', 'Raw'],
    ['derived', 'Derived'],
    ['credentials', 'Credentials'],
    ['diagnostics', 'Diagnostics'],
    ['metadata', 'Metadata'],
  ] as const)('labels a %s capability as "%s"', async (kind, label) => {
    mockList.mockResolvedValue([cap(`c-${kind}`, kind)]);
    renderWithProviders(<PrivacyPanel />);

    await waitFor(() => expect(screen.getByTestId('privacy-capability-list')).toBeInTheDocument());
    expect(within(row(`c-${kind}`)).getByText(label)).toBeInTheDocument();
  });

  it('renders all five kinds together, each with its own label', async () => {
    const kinds: DataKind[] = ['raw', 'derived', 'credentials', 'diagnostics', 'metadata'];
    mockList.mockResolvedValue(kinds.map(k => cap(`c-${k}`, k)));
    renderWithProviders(<PrivacyPanel />);

    await waitFor(() => expect(screen.getByTestId('privacy-capability-list')).toBeInTheDocument());
    for (const label of ['Raw', 'Derived', 'Credentials', 'Diagnostics', 'Metadata']) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });
});

describe('PrivacyPanel — leaves-device and destinations', () => {
  it('distinguishes a capability that leaves the device from one that does not', async () => {
    mockList.mockResolvedValue([
      cap('remote', 'derived', {
        privacy: { leaves_device: true, data_kind: 'derived', destinations: ['Backend'] },
      }),
      cap('local', 'raw', {
        privacy: { leaves_device: false, data_kind: 'raw', destinations: [] },
      }),
    ]);
    renderWithProviders(<PrivacyPanel />);

    await waitFor(() => expect(screen.getByTestId('privacy-capability-list')).toBeInTheDocument());
    expect(within(row('remote')).getByText('Leaves device')).toBeInTheDocument();
    expect(within(row('local')).getByText('Stays local')).toBeInTheDocument();
  });

  it('lists every destination, and omits the line when there are none', async () => {
    mockList.mockResolvedValue([
      cap('multi', 'derived', {
        privacy: { leaves_device: true, data_kind: 'derived', destinations: ['Alpha', 'Beta'] },
      }),
      cap('none', 'raw', { privacy: { leaves_device: false, data_kind: 'raw', destinations: [] } }),
    ]);
    renderWithProviders(<PrivacyPanel />);

    await waitFor(() => expect(screen.getByTestId('privacy-capability-list')).toBeInTheDocument());
    expect(within(row('multi')).getByText(/Alpha, Beta/)).toBeInTheDocument();
    expect(within(row('none')).queryByText(/Sent to/i)).not.toBeInTheDocument();
  });
});

describe('PrivacyPanel — catalog states', () => {
  it('says so when the catalog is empty rather than showing a bare heading', async () => {
    mockList.mockResolvedValue([]);
    renderWithProviders(<PrivacyPanel />);

    // Wait for the READY state by its own text. Waiting on
    // `expect(list).not.toBeInTheDocument()` would pass while still loading and
    // assert nothing.
    expect(
      await screen.findByText('No capabilities currently disclose data movement.')
    ).toBeInTheDocument();
    expect(screen.queryByTestId('privacy-capability-list')).not.toBeInTheDocument();
    expect(screen.queryByTestId('privacy-load-error')).not.toBeInTheDocument();
  });

  it('reaches the empty state when nothing in the catalog is annotated', async () => {
    // Unannotated entries are filtered out (`PrivacyPanel.tsx:63-65`), so a
    // full catalog with no privacy metadata is indistinguishable from an empty
    // one — and must not look like an error.
    const bare: Capability = {
      id: 'plain',
      name: 'Plain',
      domain: 'settings',
      category: 'settings',
      description: 'No privacy metadata.',
      how_to: 'Nowhere',
      status: 'stable',
    };
    mockList.mockResolvedValue([bare]);
    renderWithProviders(<PrivacyPanel />);

    expect(
      await screen.findByText('No capabilities currently disclose data movement.')
    ).toBeInTheDocument();
    expect(screen.queryByTestId('privacy-capability-list')).not.toBeInTheDocument();
    expect(screen.queryByTestId('privacy-load-error')).not.toBeInTheDocument();
  });

  it('treats a null privacy field as unannotated', async () => {
    const nulled = { ...cap('nulled', 'raw'), privacy: null } as unknown as Capability;
    mockList.mockResolvedValue([nulled]);
    renderWithProviders(<PrivacyPanel />);

    expect(
      await screen.findByText('No capabilities currently disclose data movement.')
    ).toBeInTheDocument();
    expect(screen.queryByTestId('privacy-capability-list')).not.toBeInTheDocument();
  });
});

describe('PrivacyPanel — analytics toggle', () => {
  it('does not crash, and keeps the panel usable, when persisting fails', async () => {
    mockList.mockResolvedValue([cap('c-raw', 'raw')]);
    setAnalyticsEnabledMock.mockRejectedValueOnce(new Error('config write failed'));
    renderWithProviders(<PrivacyPanel />);

    await waitFor(() => expect(screen.getByTestId('privacy-capability-list')).toBeInTheDocument());

    const toggle = screen.getByRole('switch');
    fireEvent.click(toggle);

    await waitFor(() => expect(setAnalyticsEnabledMock).toHaveBeenCalledWith(true));
    // The panel swallows the failure by design (`PrivacyPanel.tsx:81-87`); what
    // must hold is that it stays rendered rather than tearing down.
    expect(screen.getByTestId('privacy-capability-list')).toBeInTheDocument();
    expect(screen.getByRole('switch')).toBeInTheDocument();
  });
});
