import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
}));

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanGetComposioTriggerSettings: hoisted.getSettings,
  openhumanUpdateComposioTriggerSettings: hoisted.updateSettings,
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../components/SettingsHeader', () => ({
  default: ({ title }: { title: string }) => <div data-testid="settings-header">{title}</div>,
}));

async function importPanel() {
  vi.resetModules();
  const mod = await import('../ComposioTriagePanel');
  return mod.default;
}

const defaultSettings = {
  result: { triage_disabled: false, triage_disabled_toolkits: [] },
  logs: [],
};

describe('ComposioTriagePanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hoisted.getSettings.mockResolvedValue(defaultSettings);
    hoisted.updateSettings.mockResolvedValue({ result: {}, logs: [] });
  });

  test('shows loading state then renders panel on successful fetch', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    expect(screen.getByText('Loading…')).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.queryByText('Loading…')).toBeNull();
    });

    expect(screen.getByText('Integration Triggers')).toBeInTheDocument();
    expect(screen.getByLabelText('Disable AI triage for all triggers')).toBeInTheDocument();
    expect(screen.getByPlaceholderText('gmail, slack, ...')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
  });

  test('populates fields from fetched settings', async () => {
    hoisted.getSettings.mockResolvedValue({
      result: { triage_disabled: true, triage_disabled_toolkits: ['gmail', 'slack'] },
      logs: [],
    });
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.queryByText('Loading…')).toBeNull();
    });

    const toggle = screen.getByLabelText('Disable AI triage for all triggers');
    expect(toggle).toHaveAttribute('aria-checked', 'true');

    const input = screen.getByPlaceholderText('gmail, slack, ...');
    expect((input as HTMLInputElement).value).toBe('gmail, slack');
  });

  test('toggle flips aria-checked and disables the input', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.queryByText('Loading…')).toBeNull();
    });

    const toggle = screen.getByLabelText('Disable AI triage for all triggers');
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-checked', 'true');

    const input = screen.getByPlaceholderText('gmail, slack, ...');
    expect(input).toBeDisabled();
  });

  test('save calls updateSettings with correct params and shows saved status', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.queryByText('Loading…')).toBeNull();
    });

    const input = screen.getByPlaceholderText('gmail, slack, ...');
    fireEvent.change(input, { target: { value: 'Gmail, Slack' } });

    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => {
      expect(screen.getByText('Settings saved')).toBeInTheDocument();
    });

    expect(hoisted.updateSettings).toHaveBeenCalledWith({
      triage_disabled: false,
      triage_disabled_toolkits: ['gmail', 'slack'],
    });
  });

  test('shows error status when save fails', async () => {
    hoisted.updateSettings.mockRejectedValue(new Error('rpc error'));
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.queryByText('Loading…')).toBeNull();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => {
      expect(screen.getByText('Failed to save. Try again.')).toBeInTheDocument();
    });
  });

  test('handles fetch error gracefully (panel still renders)', async () => {
    hoisted.getSettings.mockRejectedValue(new Error('network error'));
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.queryByText('Loading…')).toBeNull();
    });

    // Panel still renders with defaults
    expect(screen.getByLabelText('Disable AI triage for all triggers')).toBeInTheDocument();
  });

  test('env var note is visible in description', async () => {
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.queryByText('Loading…')).toBeNull();
    });

    expect(screen.getByText('OPENHUMAN_TRIGGER_TRIAGE_DISABLED')).toBeInTheDocument();
  });
});
