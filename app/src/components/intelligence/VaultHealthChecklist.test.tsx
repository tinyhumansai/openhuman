import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, type Mock, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import type { VaultHealthCheck } from '../../utils/tauriCommands';
import { VaultHealthChecklist } from './VaultHealthChecklist';

vi.mock('../../utils/tauriCommands', () => ({ memoryTreeVaultHealthCheck: vi.fn() }));

vi.mock('../../utils/openUrl', () => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
  revealPath: vi.fn().mockResolvedValue(undefined),
}));

const { memoryTreeVaultHealthCheck } = (await import('../../utils/tauriCommands')) as unknown as {
  memoryTreeVaultHealthCheck: Mock;
};

const { openUrl, revealPath } = (await import('../../utils/openUrl')) as unknown as {
  openUrl: Mock;
  revealPath: Mock;
};

function health(overrides: Partial<VaultHealthCheck> = {}): VaultHealthCheck {
  return {
    content_root_abs: '/tmp/workspace/memory_tree/content',
    exists: true,
    readable: true,
    writable: true,
    obsidian_registered: true,
    pipeline_healthy: true,
    last_sync_ms: Date.now() - 60_000,
    ...overrides,
  };
}

describe('<VaultHealthChecklist />', () => {
  it('shows all checks as passed when the health RPC is fully healthy', async () => {
    memoryTreeVaultHealthCheck.mockResolvedValueOnce(health());
    renderWithProviders(<VaultHealthChecklist />);

    await waitFor(() => {
      expect(screen.getByTestId('vault-health-item-exists')).toBeInTheDocument();
    });
    expect(screen.getByText(/Passed · Workspace vault path exists/)).toBeInTheDocument();
    expect(screen.getByText(/Passed · Vault is writable by OpenHuman/)).toBeInTheDocument();
    expect(screen.getByText(/Passed · Vault is registered in Obsidian/)).toBeInTheDocument();
    expect(screen.getByText(/Passed · Memory pipeline is healthy/)).toBeInTheDocument();
  });

  it('surfaces a recovery hint when the vault folder is missing', async () => {
    memoryTreeVaultHealthCheck.mockResolvedValueOnce(health({ exists: false, readable: false }));
    renderWithProviders(<VaultHealthChecklist />);

    await waitFor(() => {
      expect(screen.getByTestId('vault-health-item-exists')).toBeInTheDocument();
    });
    expect(screen.getByText(/Vault folder is missing/)).toBeInTheDocument();
  });

  it('surfaces a recovery hint when the vault is not writable', async () => {
    memoryTreeVaultHealthCheck.mockResolvedValueOnce(health({ writable: false }));
    renderWithProviders(<VaultHealthChecklist />);

    await waitFor(() => {
      expect(screen.getByTestId('vault-health-item-writable')).toBeInTheDocument();
    });
    expect(screen.getByText(/cannot write to this vault yet/i)).toBeInTheDocument();
  });

  it('surfaces Obsidian registration guidance and action buttons', async () => {
    memoryTreeVaultHealthCheck.mockResolvedValueOnce(health({ obsidian_registered: false }));
    renderWithProviders(<VaultHealthChecklist />);

    await waitFor(() => {
      expect(screen.getByTestId('vault-health-item-obsidian')).toBeInTheDocument();
    });
    expect(screen.getByText(/Open folder as vault/)).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('vault-health-open-obsidian'));
    await waitFor(() => {
      expect(openUrl).toHaveBeenCalledWith(
        'obsidian://open?path=' + encodeURIComponent('/tmp/workspace/memory_tree/content')
      );
    });

    fireEvent.click(screen.getByTestId('vault-health-reveal'));
    await waitFor(() => {
      expect(revealPath).toHaveBeenCalledWith('/tmp/workspace/memory_tree/content');
    });
  });
});
