/**
 * Tests for the cross-host guard (#4278) in ObsidianVaultSection: when the
 * vault lives on a different-OS core host, "View Vault" must surface guidance
 * and disable the local-FS actions instead of firing a doomed deep link.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, type Mock, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { ObsidianVaultSection } from './ObsidianVaultSection';

vi.mock('../../utils/tauriCommands', () => ({ memoryTreeObsidianVaultStatus: vi.fn() }));

vi.mock('../../utils/tauriCommands/workspacePaths', () => ({
  resolveWorkspaceAbsolutePath: vi.fn().mockResolvedValue('/home/leigh/OHvault'),
  revealWorkspacePath: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

vi.mock('./vaultHostMatch', () => ({
  resolveVaultHostMatch: vi.fn().mockResolvedValue({ local: true }),
}));

const { memoryTreeObsidianVaultStatus } =
  (await import('../../utils/tauriCommands')) as unknown as { memoryTreeObsidianVaultStatus: Mock };

const { openUrl } = (await import('../../utils/openUrl')) as unknown as { openUrl: Mock };

const { resolveVaultHostMatch } = (await import('./vaultHostMatch')) as unknown as {
  resolveVaultHostMatch: Mock;
};

const CONTENT_ROOT = '/home/leigh/OHvault';

describe('<ObsidianVaultSection /> cross-host (#4278)', () => {
  it('surfaces guidance and disables local actions when the vault is on a different-OS core host', async () => {
    memoryTreeObsidianVaultStatus.mockResolvedValueOnce({
      registered: true,
      config_found: true,
      content_root_abs: CONTENT_ROOT,
      host_os: 'linux',
    });
    resolveVaultHostMatch.mockResolvedValueOnce({ local: false, hostOs: 'linux' });

    renderWithProviders(<ObsidianVaultSection contentRootAbs={CONTENT_ROOT} />);
    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));

    await waitFor(() => {
      expect(screen.getByTestId('obsidian-vault-guidance')).toBeInTheDocument();
    });
    expect(screen.getByTestId('obsidian-vault-guidance')).toHaveTextContent('linux');
    expect(screen.getByTestId('obsidian-reveal')).toBeDisabled();
    expect(screen.getByTestId('obsidian-open-anyway')).toBeDisabled();
    // Registered=true would normally fire the deep link; cross-host must NOT.
    expect(openUrl).not.toHaveBeenCalled();
  });

  it('fires the deep link normally for a same-host registered vault (regression)', async () => {
    memoryTreeObsidianVaultStatus.mockResolvedValueOnce({
      registered: true,
      config_found: true,
      content_root_abs: CONTENT_ROOT,
      host_os: 'macos',
    });
    resolveVaultHostMatch.mockResolvedValueOnce({ local: true, hostOs: 'macos' });

    renderWithProviders(<ObsidianVaultSection contentRootAbs={CONTENT_ROOT} />);
    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));

    await waitFor(() => {
      expect(openUrl).toHaveBeenCalledWith(
        'obsidian://open?path=' + encodeURIComponent(CONTENT_ROOT)
      );
    });
  });
});
