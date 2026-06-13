import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ToolsPanel from './ToolsPanel';

const mocks = vi.hoisted(() => ({ setOnboardingTasks: vi.fn(), useCoreStateMock: vi.fn() }));

vi.mock('../../../providers/CoreStateProvider', () => ({
  useCoreState: () => mocks.useCoreStateMock(),
}));

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (key: string) =>
      ({
        'settings.features.tools': 'Tools',
        'settings.tools.chooseCapabilities': 'Choose capabilities',
        'settings.tools.saveChanges': 'Save Changes',
        'settings.tools.preferencesSaved': 'Preferences saved',
        'settings.tools.saveFailed': 'Unable to save preferences',
      })[key] ?? key,
  }),
}));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ breadcrumbs: [], navigateBack: vi.fn() }),
}));

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));

function coreState(enabledTools: string[]) {
  return {
    snapshot: {
      localState: {
        onboardingTasks: {
          accessibilityPermissionGranted: false,
          localModelConsentGiven: false,
          localModelDownloadStarted: false,
          enabledTools,
          connectedSources: [],
        },
      },
    },
    setOnboardingTasks: mocks.setOnboardingTasks,
  };
}

describe('<ToolsPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.useCoreStateMock.mockReturnValue(coreState(['shell']));
    mocks.setOnboardingTasks.mockResolvedValue(undefined);
  });

  it('exposes tool toggle state and saves the updated enabled tools list', async () => {
    render(<ToolsPanel />);

    const shellToggle = screen.getByRole('switch', { name: /Shell Commands/ });
    await waitFor(() => expect(shellToggle).toHaveAttribute('aria-checked', 'true'));

    fireEvent.click(shellToggle);

    expect(shellToggle).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(screen.getByRole('button', { name: 'Save Changes' }));

    await waitFor(() =>
      expect(mocks.setOnboardingTasks).toHaveBeenCalledWith(
        expect.objectContaining({ enabledTools: [] })
      )
    );
  });

  it('renders SettingsHeader when embedded=false (line 110)', () => {
    // Default embedded=false shows the header
    render(<ToolsPanel embedded={false} />);
    expect(screen.getByText('Tools')).toBeInTheDocument();
  });

  it('does not render SettingsHeader when embedded=true (line 101-108 skipped)', () => {
    // When embedded, the header section is not rendered
    render(<ToolsPanel embedded={true} />);
    // The header mock outputs a <h1> with the title — embedded should NOT show it
    expect(screen.queryByRole('heading', { name: 'Tools' })).not.toBeInTheDocument();
  });

  it('shows Save Changes button after toggling a tool (dirty state, line 145-155)', async () => {
    render(<ToolsPanel />);

    const shellToggle = screen.getByRole('switch', { name: /Shell Commands/ });
    await waitFor(() => expect(shellToggle).toHaveAttribute('aria-checked', 'true'));

    // Before toggle — no Save button
    expect(screen.queryByRole('button', { name: 'Save Changes' })).not.toBeInTheDocument();

    // After toggle — dirty=true → Save Changes appears (line 145-155)
    fireEvent.click(shellToggle);
    expect(screen.getByRole('button', { name: 'Save Changes' })).toBeInTheDocument();
  });
});
