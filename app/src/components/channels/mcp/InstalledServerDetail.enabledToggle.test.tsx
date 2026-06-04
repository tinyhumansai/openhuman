import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import InstalledServerDetail from './InstalledServerDetail';

const mockConnect = vi.fn();
const mockDisconnect = vi.fn();
const mockUninstall = vi.fn();
const mockUpdateEnv = vi.fn();
const mockSetEnabled = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    connect: (...args: unknown[]) => mockConnect(...args),
    disconnect: (...args: unknown[]) => mockDisconnect(...args),
    uninstall: (...args: unknown[]) => mockUninstall(...args),
    updateEnv: (...args: unknown[]) => mockUpdateEnv(...args),
    setEnabled: (...args: unknown[]) => mockSetEnabled(...args),
    configAssist: vi.fn(),
  },
}));

const BASE_SERVER_ENABLED = {
  server_id: 'srv-1',
  qualified_name: 'acme/test-server',
  display_name: 'Test Server',
  description: 'A test MCP server',
  command_kind: 'node' as const,
  command: 'node',
  args: [],
  env_keys: [] as string[],
  installed_at: 1_700_000_000,
  enabled: true,
};

const BASE_SERVER_DISABLED = { ...BASE_SERVER_ENABLED, enabled: false };

describe('InstalledServerDetail — enable/disable toggle', () => {
  beforeEach(() => {
    mockConnect.mockReset();
    mockDisconnect.mockReset();
    mockUninstall.mockReset();
    mockUpdateEnv.mockReset();
    mockSetEnabled.mockReset();
  });

  it('shows Disable button when server is enabled', () => {
    render(
      <InstalledServerDetail
        server={BASE_SERVER_ENABLED}
        connStatus={undefined}
        onUninstalled={() => {}}
      />
    );
    expect(screen.getByRole('button', { name: /disable/i })).toBeInTheDocument();
  });

  it('shows Enable button when server is disabled and hides Connect button', () => {
    render(
      <InstalledServerDetail
        server={BASE_SERVER_DISABLED}
        connStatus={undefined}
        onUninstalled={() => {}}
      />
    );
    expect(screen.getByRole('button', { name: /^enable$/i })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^connect$/i })).not.toBeInTheDocument();
  });

  it('calls setEnabled(false) on Disable click and notifies parent via onEnabledChange', async () => {
    mockSetEnabled.mockResolvedValue({ server_id: 'srv-1', enabled: false });
    const onEnabledChange = vi.fn();
    render(
      <InstalledServerDetail
        server={BASE_SERVER_ENABLED}
        connStatus={undefined}
        onUninstalled={() => {}}
        onEnabledChange={onEnabledChange}
      />
    );

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /disable/i }));
    });

    await waitFor(() => {
      expect(mockSetEnabled).toHaveBeenCalledWith('srv-1', false);
      expect(onEnabledChange).toHaveBeenCalledWith('srv-1', false);
    });
  });

  it('surfaces API error inline if setEnabled rejects', async () => {
    mockSetEnabled.mockRejectedValue(new Error('Server unavailable'));
    render(
      <InstalledServerDetail
        server={BASE_SERVER_ENABLED}
        connStatus={undefined}
        onUninstalled={() => {}}
      />
    );

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /disable/i }));
    });

    await waitFor(() => expect(screen.getByText('Server unavailable')).toBeInTheDocument());
  });
});
