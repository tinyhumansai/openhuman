/**
 * Webhooks page — unit tests covering changed lines.
 *
 * Target lines: 12, 18-19, 56, 62, 65, 67, 85, 93
 *
 * 12  — debug('settings:webhooks') / log call
 * 18-19 — destructuring useSettingsNavigation + useComposeioTriggerHistory
 * 56  — coreConnected conditional class on status badge
 * 62  — coreConnected conditional class on dot span
 * 65  — connected/disconnect label rendering
 * 67  — Refresh button triggers refresh()
 * 85  — error banner rendered when error is non-null
 * 93  — currentDayFile displayed (or loading fallback)
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import React from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Webhooks from '../Webhooks';

// ── Mocks ─────────────────────────────────────────────────────────────────────

vi.mock('../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (key: string) => {
      const map: Record<string, string> = {
        'settings.developerMenu.composeioTriggers.title': 'Webhook Triggers',
        'common.loading': 'Loading...',
        'common.refresh': 'Refresh',
        'skills.connected': 'Connected',
        'skills.disconnect': 'Disconnected',
        'webhooks.archiveDirectory': 'Archive Directory',
        'webhooks.todayFile': "Today's File",
        'skills.search': 'Search',
        'misc.rehydrating': 'Rehydrating...',
      };
      return map[key] ?? key;
    },
  }),
}));

const mockNavigateBack = vi.fn();

vi.mock('../../components/settings/hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: mockNavigateBack, breadcrumbs: [] }),
}));

vi.mock('../../components/settings/components/SettingsHeader', () => ({
  default: ({
    title,
    action,
  }: {
    title: string;
    action?: React.ReactNode;
    showBackButton?: boolean;
    onBack?: () => void;
    breadcrumbs?: unknown[];
  }) => (
    <div data-testid="settings-header">
      <h1>{title}</h1>
      {action && <div data-testid="header-action">{action}</div>}
    </div>
  ),
}));

vi.mock('../../components/settings/controls', () => ({
  SettingsSection: ({
    children,
    title,
    description,
  }: {
    children: React.ReactNode;
    title?: string;
    description?: string;
  }) => (
    <div>
      {title && <h2>{title}</h2>}
      {description && <p>{description}</p>}
      {children}
    </div>
  ),
}));

vi.mock('../../components/webhooks/ComposeioTriggerHistory', () => ({
  default: ({ entries }: { entries: unknown[] }) => (
    <div data-testid="trigger-history">{entries.length} entries</div>
  ),
}));

vi.mock('../../components/settings/panels/ComposioTriagePanel', () => ({
  default: () => <div data-testid="composio-triage-panel" />,
}));

const mockRefresh = vi.fn();

const defaultHookState = {
  archiveDir: '/path/to/archive',
  currentDayFile: '/path/to/archive/2026-06-10.jsonl',
  entries: [],
  loading: false,
  error: null,
  coreConnected: true,
  refresh: mockRefresh,
};

vi.mock('../../hooks/useComposeioTriggerHistory', () => ({
  useComposeioTriggerHistory: () => mockUseComposeioTriggerHistory(),
}));

const mockUseComposeioTriggerHistory = vi.fn();

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('Webhooks page', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUseComposeioTriggerHistory.mockReturnValue(defaultHookState);
    mockRefresh.mockResolvedValue(undefined);
  });

  it('renders the page title and header (lines 12, 18-19)', () => {
    render(<Webhooks />);
    expect(screen.getByText('Webhook Triggers')).toBeInTheDocument();
    expect(screen.getByTestId('settings-header')).toBeInTheDocument();
  });

  it('shows connected status badge when coreConnected=true (lines 56, 62, 65)', () => {
    render(<Webhooks />);
    // Connected label is rendered (line 65)
    expect(screen.getByText('Connected')).toBeInTheDocument();
  });

  it('shows disconnected status badge when coreConnected=false (lines 56, 62, 65)', () => {
    mockUseComposeioTriggerHistory.mockReturnValue({ ...defaultHookState, coreConnected: false });
    render(<Webhooks />);
    expect(screen.getByText('Disconnected')).toBeInTheDocument();
  });

  it('clicking Refresh button calls refresh() (line 67)', async () => {
    render(<Webhooks />);

    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));

    await waitFor(() => {
      expect(mockRefresh).toHaveBeenCalledTimes(1);
    });
  });

  it('renders error banner when error is non-null (line 85)', () => {
    mockUseComposeioTriggerHistory.mockReturnValue({
      ...defaultHookState,
      error: 'Failed to connect to core',
    });
    render(<Webhooks />);
    expect(screen.getByText('Failed to connect to core')).toBeInTheDocument();
  });

  it('shows the archive directory path (line 85 area / archiveDir)', () => {
    render(<Webhooks />);
    expect(screen.getByText('/path/to/archive')).toBeInTheDocument();
  });

  it('shows the current day file path (line 93)', () => {
    render(<Webhooks />);
    expect(screen.getByText('/path/to/archive/2026-06-10.jsonl')).toBeInTheDocument();
  });

  it('shows Loading... when currentDayFile is null (line 93 fallback)', () => {
    mockUseComposeioTriggerHistory.mockReturnValue({
      ...defaultHookState,
      currentDayFile: null,
      archiveDir: null,
    });
    render(<Webhooks />);
    // Both archiveDir and currentDayFile fall back to t('common.loading')
    const loadingEls = screen.getAllByText('Loading...');
    expect(loadingEls.length).toBeGreaterThanOrEqual(2);
  });

  it('shows loading spinner when loading=true and entries is empty (line 23)', () => {
    mockUseComposeioTriggerHistory.mockReturnValue({
      ...defaultHookState,
      loading: true,
      entries: [],
    });
    render(<Webhooks />);
    // The loading spinner branch is rendered (no header action or content)
    const header = screen.getByTestId('settings-header');
    expect(header).toBeInTheDocument();
    // Spinner text shown
    expect(screen.getByText('Loading...')).toBeInTheDocument();
  });

  it('renders the trigger history component with entries count', () => {
    mockUseComposeioTriggerHistory.mockReturnValue({
      ...defaultHookState,
      entries: [{ id: 1 }, { id: 2 }],
    });
    render(<Webhooks />);
    expect(screen.getByTestId('trigger-history')).toHaveTextContent('2 entries');
  });
});
