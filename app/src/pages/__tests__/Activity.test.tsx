import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// ---------------------------------------------------------------------------
// Stubs — keep the test fast by mocking every heavy dependency.
// ---------------------------------------------------------------------------

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

vi.mock('../../components/intelligence/IntelligenceSubconsciousTab', () => ({
  default: () => <div data-testid="tab-backgroundActivity" />,
}));
vi.mock('../../components/intelligence/WorkflowsTab', () => ({
  default: () => <div data-testid="tab-automations" />,
}));
vi.mock('../../components/intelligence/Toast', () => ({ ToastContainer: () => null }));
vi.mock('../../components/intelligence/ConfirmationModal', () => ({
  ConfirmationModal: () => null,
}));

// Stub Notifications so the Alerts tab renders a predictable sentinel without
// needing a Redux store, router context beyond MemoryRouter, etc.
vi.mock('../Notifications', () => ({ default: () => <div data-testid="tab-alerts" /> }));

vi.mock('../../hooks/useIntelligenceSocket', () => ({
  useIntelligenceSocket: () => ({ isConnected: true }),
  useIntelligenceSocketManager: () => ({ connect: vi.fn() }),
}));
vi.mock('../../hooks/useSubconscious', () => ({
  useSubconscious: () => ({
    status: 'idle',
    mode: 'manual',
    intervalMinutes: 30,
    triggering: false,
    settingMode: false,
    triggerTick: vi.fn(),
    setMode: vi.fn(),
    setIntervalMinutes: vi.fn(),
  }),
}));

// Dynamic import AFTER all mocks are in place (same pattern as original test).
const Activity = (await import('../Activity')).default;

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Activity />
    </MemoryRouter>
  );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Activity URL-backed tab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('defaults to the automations tab when no ?tab is present', async () => {
    renderAt('/activity');
    await waitFor(() => expect(screen.getByTestId('tab-automations')).toBeInTheDocument());
  });

  it('honours ?tab=automations from the URL', async () => {
    renderAt('/activity?tab=automations');
    await waitFor(() => expect(screen.getByTestId('tab-automations')).toBeInTheDocument());
  });

  it('honours ?tab=backgroundActivity from the URL', async () => {
    renderAt('/activity?tab=backgroundActivity');
    await waitFor(() => expect(screen.getByTestId('tab-backgroundActivity')).toBeInTheDocument());
  });

  it('honours ?tab=alerts from the URL', async () => {
    renderAt('/activity?tab=alerts');
    await waitFor(() => expect(screen.getByTestId('tab-alerts')).toBeInTheDocument());
  });

  it('falls back to automations for an unknown ?tab value', async () => {
    renderAt('/activity?tab=bogus');
    await waitFor(() => expect(screen.getByTestId('tab-automations')).toBeInTheDocument());
  });

  // Back-compat: old deep links (?tab=tasks|memory|agents|council) are no longer
  // visible Activity tabs — they should fall back to automations rather than error.
  it('falls back to automations for ?tab=tasks (relocated to Settings → Developer Options)', async () => {
    renderAt('/activity?tab=tasks');
    await waitFor(() => expect(screen.getByTestId('tab-automations')).toBeInTheDocument());
    expect(screen.queryByTestId('tab-tasks')).not.toBeInTheDocument();
  });

  it('falls back to automations for ?tab=memory (relocated to Settings)', async () => {
    renderAt('/activity?tab=memory');
    await waitFor(() => expect(screen.getByTestId('tab-automations')).toBeInTheDocument());
    expect(screen.queryByTestId('tab-memory')).not.toBeInTheDocument();
  });

  it('falls back to automations for ?tab=agents (relocated to Settings)', async () => {
    renderAt('/activity?tab=agents');
    await waitFor(() => expect(screen.getByTestId('tab-automations')).toBeInTheDocument());
    expect(screen.queryByTestId('tab-agents')).not.toBeInTheDocument();
  });

  it('falls back to automations for ?tab=council (relocated to Settings)', async () => {
    renderAt('/activity?tab=council');
    await waitFor(() => expect(screen.getByTestId('tab-automations')).toBeInTheDocument());
    expect(screen.queryByTestId('tab-council')).not.toBeInTheDocument();
  });

  it('renders the intelligence-header data-walkthrough anchor (non-alerts tabs)', async () => {
    renderAt('/activity');
    await waitFor(() =>
      expect(document.querySelector('[data-walkthrough="intelligence-header"]')).not.toBeNull()
    );
  });
});

describe('Activity tab — tab set', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders exactly three tab pills: automations, backgroundActivity, alerts', async () => {
    renderAt('/activity');
    await waitFor(() => screen.getByTestId('tab-automations'));
    // Each label key is returned as-is by the stub t() function.
    expect(screen.getAllByText('activity.tabs.automations').length).toBeGreaterThan(0);
    expect(screen.getAllByText('activity.tabs.backgroundActivity').length).toBeGreaterThan(0);
    expect(screen.getAllByText('activity.tabs.alerts').length).toBeGreaterThan(0);
  });

  it('does not render tasks, memory, agents, or council pills', async () => {
    renderAt('/activity');
    await waitFor(() => screen.getByTestId('tab-automations'));
    expect(screen.queryByText('memory.tab.tasks')).not.toBeInTheDocument();
    expect(screen.queryByText('memory.tab.memory')).not.toBeInTheDocument();
    expect(screen.queryByText('memory.tab.agents')).not.toBeInTheDocument();
    expect(screen.queryByText('memory.tab.council')).not.toBeInTheDocument();
  });

  it('clicking the backgroundActivity pill switches to the backgroundActivity tab', async () => {
    renderAt('/activity');
    await waitFor(() => screen.getAllByText('activity.tabs.backgroundActivity'));
    fireEvent.click(screen.getAllByText('activity.tabs.backgroundActivity')[0]);
    await waitFor(() => expect(screen.getByTestId('tab-backgroundActivity')).toBeInTheDocument());
  });

  it('clicking the alerts pill switches to the alerts tab', async () => {
    renderAt('/activity');
    await waitFor(() => screen.getAllByText('activity.tabs.alerts'));
    fireEvent.click(screen.getAllByText('activity.tabs.alerts')[0]);
    await waitFor(() => expect(screen.getByTestId('tab-alerts')).toBeInTheDocument());
  });

  it('alerts tab renders the Notifications component', async () => {
    renderAt('/activity?tab=alerts');
    await waitFor(() => expect(screen.getByTestId('tab-alerts')).toBeInTheDocument());
    // The card wrapper is NOT rendered in the alerts tab.
    expect(screen.queryByTestId('tab-automations')).not.toBeInTheDocument();
  });
});
