/**
 * Tests for the staging-only "Trigger Sentry Test" row that
 * `DeveloperOptionsPanel` renders at the top when
 * `APP_ENVIRONMENT === 'staging'`. Covers visibility gating, the
 * idle/sending/sent/error state machine, and the failure path.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({
  trigger: vi.fn(),
  appEnvironment: 'staging' as 'staging' | 'production' | 'development',
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: hoisted.invoke, isTauri: hoisted.isTauri }));

vi.mock('../../../../services/analytics', () => ({ triggerSentryTestEvent: hoisted.trigger }));

vi.mock('../../../../utils/config', () => ({
  get APP_ENVIRONMENT() {
    return hoisted.appEnvironment;
  },
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateToSettings: vi.fn(),
    navigateBack: vi.fn(),
    breadcrumbs: [],
  }),
}));

async function importPanel() {
  const mod = await import('../DeveloperOptionsPanel');
  return mod.default;
}

describe('DeveloperOptionsPanel — Sentry test row', () => {
  beforeEach(() => {
    hoisted.trigger.mockReset();
    // The panel always renders LogsFolderRow, which fires
    // `invoke('logs_folder_path')` on mount. Stub it to a resolved no-op
    // so this suite's tests focus on the Sentry row without unhandled
    // rejections from the App-logs effect.
    hoisted.invoke.mockReset();
    hoisted.invoke.mockResolvedValue(null);
    hoisted.isTauri.mockReset();
    hoisted.isTauri.mockReturnValue(true);
    hoisted.appEnvironment = 'staging';
  });

  test('does not render the row in production builds', async () => {
    hoisted.appEnvironment = 'production';
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    expect(screen.queryByText(/Trigger Sentry Test/i)).toBeNull();
    expect(screen.queryByRole('button', { name: /Send test event/i })).toBeNull();
  });

  test('renders the staging row and fires the helper on click', async () => {
    hoisted.trigger.mockResolvedValue('event-id-xyz');
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    expect(screen.getByText(/Trigger Sentry Test/i)).toBeInTheDocument();
    const button = screen.getByRole('button', { name: /Send test event/i });

    fireEvent.click(button);

    await waitFor(() => {
      expect(hoisted.trigger).toHaveBeenCalledTimes(1);
    });

    await waitFor(() => {
      expect(screen.getByText(/Event sent\./i)).toBeInTheDocument();
    });
    expect(screen.getByText(/event-id-xyz/)).toBeInTheDocument();

    // Status updates must announce via an accessible live region — without
    // role="status" + aria-live, screen readers stay silent on click.
    const live = screen.getByRole('status');
    expect(live).toHaveAttribute('aria-live', 'polite');
    expect(live).toHaveTextContent(/Event sent.*event-id-xyz/);
  });

  test('shows "no id" branch when the helper resolves with undefined', async () => {
    hoisted.trigger.mockResolvedValue(undefined);
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: /Send test event/i }));

    await waitFor(() => {
      expect(screen.getByText(/Event sent\./i)).toBeInTheDocument();
    });
    expect(screen.getByText(/no id/i)).toBeInTheDocument();
  });

  test('surfaces the error message when the helper throws', async () => {
    hoisted.trigger.mockRejectedValue(new Error('network broke'));
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: /Send test event/i }));

    await waitFor(() => {
      expect(screen.getByText(/Failed: network broke/i)).toBeInTheDocument();
    });
  });
});

describe('DeveloperOptionsPanel — App logs row', () => {
  beforeEach(() => {
    hoisted.invoke.mockReset();
    hoisted.isTauri.mockReset();
    hoisted.isTauri.mockReturnValue(true);
    // Force production so the staging Sentry row stays hidden and we
    // assert against the App logs row in isolation.
    hoisted.appEnvironment = 'production';
  });

  test('renders nothing when not running under Tauri', async () => {
    hoisted.isTauri.mockReturnValue(false);
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);
    expect(screen.queryByText(/App logs/i)).toBeNull();
    expect(screen.queryByRole('button', { name: /Open logs folder/i })).toBeNull();
  });

  test('shows the resolved log path on mount', async () => {
    hoisted.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'logs_folder_path') return Promise.resolve('/tmp/openhuman/logs');
      return Promise.resolve();
    });
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText('/tmp/openhuman/logs')).toBeInTheDocument();
    });
    expect(hoisted.invoke).toHaveBeenCalledWith('logs_folder_path');
  });

  test('invokes reveal_logs_folder on click', async () => {
    hoisted.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'logs_folder_path') return Promise.resolve(null);
      if (cmd === 'reveal_logs_folder') return Promise.resolve();
      return Promise.resolve();
    });
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: /Open logs folder/i }));

    await waitFor(() => {
      expect(hoisted.invoke).toHaveBeenCalledWith('reveal_logs_folder');
    });
  });

  test('surfaces the reveal error message in the live region', async () => {
    hoisted.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'logs_folder_path') return Promise.resolve(null);
      if (cmd === 'reveal_logs_folder')
        return Promise.reject(new Error('log directory not initialized'));
      return Promise.resolve();
    });
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: /Open logs folder/i }));

    await waitFor(() => {
      expect(screen.getByText(/log directory not initialized/i)).toBeInTheDocument();
    });
    const live = screen.getByRole('status');
    expect(live).toHaveAttribute('aria-live', 'polite');
  });

  test('surfaces the path-resolve error when logs_folder_path rejects', async () => {
    hoisted.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'logs_folder_path') return Promise.reject(new Error('boom'));
      return Promise.resolve();
    });
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText(/boom/i)).toBeInTheDocument();
    });
  });
});
