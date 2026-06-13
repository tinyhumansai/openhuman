import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../../services/coreRpcClient';
import { renderWithProviders } from '../../../../test/test-utils';
import CompanionPanel from '../CompanionPanel';

vi.mock('../../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    breadcrumbs: [{ label: 'Settings' }, { label: 'Features' }],
  }),
}));

const callCoreRpcMock = callCoreRpc as unknown as ReturnType<typeof vi.fn>;

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

const mockConfig = {
  hotkey: 'ctrl+space',
  activation_mode: 'push',
  ttl_secs: 3600,
  capture_screen: true,
  include_app_context: true,
};

beforeEach(() => {
  vi.clearAllMocks();
  callCoreRpcMock.mockImplementation(async ({ method }: { method: string }) => {
    if (method === 'openhuman.companion_status') return mockStatus;
    if (method === 'openhuman.companion_config_get') return mockConfig;
    throw new Error(`unmocked method: ${method}`);
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
    callCoreRpcMock.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.companion_status')
        return {
          ...mockStatus,
          active: true,
          state: 'listening',
          session_id: 'sess-123',
          turn_count: 3,
          remaining_ms: 300000,
        };
      if (method === 'openhuman.companion_config_get') return mockConfig;
      throw new Error(`unmocked method: ${method}`);
    });

    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getByText('Stop Session')).toBeInTheDocument();
    });
  });

  it('calls companion_start_session when start button clicked', async () => {
    const user = userEvent.setup();
    renderWithProviders(<CompanionPanel />);

    await waitFor(() => {
      expect(screen.getByText('Start Session')).toBeInTheDocument();
    });

    callCoreRpcMock.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.companion_start_session')
        return { session_id: 'new-sess', state: 'idle', expires_at_ms: null };
      if (method === 'openhuman.companion_status') return mockStatus;
      if (method === 'openhuman.companion_config_get') return mockConfig;
      throw new Error(`unmocked method: ${method}`);
    });

    await user.click(screen.getByText('Start Session'));

    await waitFor(() => {
      expect(callCoreRpcMock).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.companion_start_session' })
      );
    });
  });

  it('shows error when start session fails', async () => {
    const user = userEvent.setup();
    renderWithProviders(<CompanionPanel />);

    await waitFor(() => {
      expect(screen.getByText('Start Session')).toBeInTheDocument();
    });

    callCoreRpcMock.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.companion_start_session') throw new Error('consent required');
      if (method === 'openhuman.companion_status') return mockStatus;
      if (method === 'openhuman.companion_config_get') return mockConfig;
      throw new Error(`unmocked method: ${method}`);
    });

    await user.click(screen.getByText('Start Session'));

    await waitFor(() => {
      expect(screen.getByText('consent required')).toBeInTheDocument();
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
      expect(callCoreRpcMock).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.companion_status' })
      );
    });
  });

  it('shows error when companion_status fetch fails', async () => {
    callCoreRpcMock.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.companion_status') throw new Error('rpc down');
      if (method === 'openhuman.companion_config_get') return mockConfig;
      throw new Error(`unmocked method: ${method}`);
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
    callCoreRpcMock.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.companion_status') return currentStatus;
      if (method === 'openhuman.companion_config_get') return mockConfig;
      if (method === 'openhuman.companion_stop_session') {
        currentStatus = mockStatus;
        return { stopped: true, reason: 'user_requested' };
      }
      throw new Error(`unmocked method: ${method}`);
    });

    const user = userEvent.setup();
    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getByText('Stop Session')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Stop Session'));

    await waitFor(() => {
      expect(callCoreRpcMock).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.companion_stop_session' })
      );
    });
  });

  it('shows error when stop session fails', async () => {
    const activeStatus = {
      ...mockStatus,
      active: true,
      state: 'speaking' as const,
      session_id: 'sess-active',
    };
    callCoreRpcMock.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.companion_status') return activeStatus;
      if (method === 'openhuman.companion_config_get') return mockConfig;
      if (method === 'openhuman.companion_stop_session') throw new Error('cannot stop');
      throw new Error(`unmocked method: ${method}`);
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

  it('renders "Disabled" when capture_screen and include_app_context are false', async () => {
    callCoreRpcMock.mockImplementation(async ({ method }: { method: string }) => {
      if (method === 'openhuman.companion_status') return mockStatus;
      if (method === 'openhuman.companion_config_get') {
        return { ...mockConfig, capture_screen: false, include_app_context: false };
      }
      throw new Error(`unmocked method: ${method}`);
    });
    renderWithProviders(<CompanionPanel />);
    await waitFor(() => {
      expect(screen.getAllByText('Disabled').length).toBeGreaterThanOrEqual(2);
    });
  });
});
