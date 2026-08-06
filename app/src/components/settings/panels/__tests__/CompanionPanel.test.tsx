import { invoke } from '@tauri-apps/api/core';
import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import CompanionPanel from '../CompanionPanel';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

// The panel guards its calls behind isTauri(); force it on in the test env.
vi.mock('../../../../utils/tauriCommands/common', () => ({ isTauri: () => true }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    breadcrumbs: [{ label: 'Settings' }, { label: 'Features' }],
  }),
}));

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

const mockStatus = {
  active: false,
  state: 'idle' as const,
  session_id: null,
  started_at_ms: null,
  expires_at_ms: null,
  remaining_ms: null,
  turn_count: 0,
  last_error: null,
};

const mockConfig = { hotkey: 'ctrl+space', activation_mode: 'push', ttl_secs: 3600 };

beforeEach(() => {
  vi.clearAllMocks();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === 'companion_status') return mockStatus;
    if (cmd === 'companion_config_get') return mockConfig;
    throw new Error(`unmocked command: ${cmd}`);
  });
});

describe('CompanionPanel', () => {
  it('renders idle state when session is inactive', async () => {
    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getByText('Start Session')).toBeInTheDocument();
    });
    expect(screen.getByText('Inactive')).toBeInTheDocument();
  });

  it('renders active state when session is active', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'companion_status')
        return {
          ...mockStatus,
          active: true,
          state: 'listening',
          session_id: 'sess-123',
          turn_count: 3,
          remaining_ms: 300000,
        };
      if (cmd === 'companion_config_get') return mockConfig;
      throw new Error(`unmocked command: ${cmd}`);
    });

    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getByText('Stop Session')).toBeInTheDocument();
    });
  });

  it('starts a session and registers its configured hotkey', async () => {
    const user = userEvent.setup();
    renderWithProviders(<CompanionPanel />);

    await waitFor(() => {
      expect(screen.getByText('Start Session')).toBeInTheDocument();
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'companion_start_session')
        return { session_id: 'new-sess', state: 'idle', expires_at_ms: null };
      if (cmd === 'register_companion_hotkey') return undefined;
      if (cmd === 'companion_status') return mockStatus;
      if (cmd === 'companion_config_get') return mockConfig;
      throw new Error(`unmocked command: ${cmd}`);
    });

    await user.click(screen.getByText('Start Session'));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('companion_start_session', { consent: true });
      expect(invokeMock).toHaveBeenCalledWith('register_companion_hotkey', {
        shortcut: mockConfig.hotkey,
      });
    });

    const calls = invokeMock.mock.calls.map(([command]) => command);
    expect(calls.indexOf('companion_start_session')).toBeLessThan(
      calls.indexOf('register_companion_hotkey')
    );
  });

  it('shows error when start session fails', async () => {
    const user = userEvent.setup();
    renderWithProviders(<CompanionPanel />);

    await waitFor(() => {
      expect(screen.getByText('Start Session')).toBeInTheDocument();
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'companion_start_session') throw new Error('consent required');
      if (cmd === 'companion_status') return mockStatus;
      if (cmd === 'companion_config_get') return mockConfig;
      throw new Error(`unmocked command: ${cmd}`);
    });

    await user.click(screen.getByText('Start Session'));

    await waitFor(() => {
      expect(screen.getByText('consent required')).toBeInTheDocument();
    });
  });

  it('stops the new session when hotkey registration fails', async () => {
    const user = userEvent.setup();
    renderWithProviders(<CompanionPanel />);

    await waitFor(() => {
      expect(screen.getByText('Start Session')).toBeInTheDocument();
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'companion_start_session')
        return { session_id: 'new-sess', state: 'idle', expires_at_ms: null };
      if (cmd === 'register_companion_hotkey') throw new Error('shortcut unavailable');
      if (cmd === 'companion_stop_session') return { stopped: true, reason: 'user_requested' };
      if (cmd === 'companion_status') return mockStatus;
      if (cmd === 'companion_config_get') return mockConfig;
      throw new Error(`unmocked command: ${cmd}`);
    });

    await user.click(screen.getByText('Start Session'));

    await waitFor(() => {
      expect(screen.getByText('shortcut unavailable')).toBeInTheDocument();
      expect(invokeMock).toHaveBeenCalledWith('companion_stop_session');
    });
  });

  it('displays config values', async () => {
    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getByText('ctrl+space')).toBeInTheDocument();
    });
    expect(screen.getByText('push')).toBeInTheDocument();
    expect(screen.getByText('3600s')).toBeInTheDocument();
  });

  it('calls companion_status on mount', async () => {
    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('companion_status');
    });
  });

  it('shows error when companion_status fetch fails', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'companion_status') throw new Error('rpc down');
      if (cmd === 'companion_config_get') return mockConfig;
      throw new Error(`unmocked command: ${cmd}`);
    });
    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getByText('rpc down')).toBeInTheDocument();
    });
  });

  it('stops an active session via companion_stop_session', async () => {
    const activeStatus = {
      ...mockStatus,
      active: true,
      state: 'listening' as const,
      session_id: 'sess-active',
    };
    let currentStatus: typeof mockStatus | typeof activeStatus = activeStatus;
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'companion_status') return currentStatus;
      if (cmd === 'companion_config_get') return mockConfig;
      if (cmd === 'companion_stop_session') {
        currentStatus = mockStatus;
        return { stopped: true, reason: 'user_requested' };
      }
      throw new Error(`unmocked command: ${cmd}`);
    });

    const user = userEvent.setup();
    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getByText('Stop Session')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Stop Session'));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('companion_stop_session');
    });
  });

  it('shows error when stop session fails', async () => {
    const activeStatus = {
      ...mockStatus,
      active: true,
      state: 'speaking' as const,
      session_id: 'sess-active',
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'companion_status') return activeStatus;
      if (cmd === 'companion_config_get') return mockConfig;
      if (cmd === 'companion_stop_session') throw new Error('cannot stop');
      throw new Error(`unmocked command: ${cmd}`);
    });

    const user = userEvent.setup();
    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getByText('Stop Session')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Stop Session'));

    await waitFor(() => {
      expect(screen.getByText('cannot stop')).toBeInTheDocument();
    });
  });

  it('does not render screen capture or app context configuration', async () => {
    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getByText('ctrl+space')).toBeInTheDocument();
    });
    expect(screen.queryByText('Screen Capture')).not.toBeInTheDocument();
    expect(screen.queryByText('App Context')).not.toBeInTheDocument();
  });
});
