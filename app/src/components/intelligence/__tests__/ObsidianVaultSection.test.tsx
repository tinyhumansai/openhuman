import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { ObsidianVaultSection } from '../ObsidianVaultSection';

vi.mock('../../../utils/tauriCommands', () => ({ memoryTreeObsidianVaultStatus: vi.fn() }));

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

vi.mock('../../../utils/tauriCommands/workspacePaths', () => ({
  revealWorkspacePath: vi.fn().mockResolvedValue(undefined),
  resolveWorkspaceAbsolutePath: vi.fn().mockResolvedValue('/tmp/workspace/memory_tree/content'),
}));

const { memoryTreeObsidianVaultStatus } =
  (await import('../../../utils/tauriCommands')) as unknown as {
    memoryTreeObsidianVaultStatus: Mock;
  };

const { openUrl } = (await import('../../../utils/openUrl')) as unknown as { openUrl: Mock };

const { revealWorkspacePath, resolveWorkspaceAbsolutePath } =
  (await import('../../../utils/tauriCommands/workspacePaths')) as unknown as {
    revealWorkspacePath: Mock;
    resolveWorkspaceAbsolutePath: Mock;
  };

const ROOT = '/tmp/workspace/memory_tree/content';
const DEEP_LINK = 'obsidian://open?path=' + encodeURIComponent(ROOT);

function status(over: Partial<{ registered: boolean; config_found: boolean }> = {}) {
  return { registered: false, config_found: true, content_root_abs: ROOT, ...over };
}

describe('ObsidianVaultSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    openUrl.mockResolvedValue(undefined);
    revealWorkspacePath.mockResolvedValue(undefined);
    resolveWorkspaceAbsolutePath.mockResolvedValue(ROOT);
  });

  it('registered vault → opens the deep link directly, no guidance shown', async () => {
    memoryTreeObsidianVaultStatus.mockResolvedValue(status({ registered: true }));
    const onToast = vi.fn();
    renderWithProviders(<ObsidianVaultSection contentRootAbs={ROOT} onToast={onToast} />);

    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));

    await waitFor(() => expect(openUrl).toHaveBeenCalledWith(DEEP_LINK));
    expect(screen.queryByTestId('obsidian-vault-guidance')).toBeNull();
    await waitFor(() => expect(onToast).toHaveBeenCalled());
    expect(onToast.mock.calls[0][0].type).toBe('info');
  });

  it('unregistered vault → no deep link, shows guidance with the vault path', async () => {
    memoryTreeObsidianVaultStatus.mockResolvedValue(status());
    renderWithProviders(<ObsidianVaultSection contentRootAbs={ROOT} />);

    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));

    await waitFor(() => expect(screen.getByTestId('obsidian-vault-guidance')).toBeInTheDocument());
    expect(openUrl).not.toHaveBeenCalled();
    expect(screen.getByTestId('obsidian-vault-path')).toHaveTextContent(ROOT);
  });

  it('"Open anyway" fires the deep link even when unregistered', async () => {
    memoryTreeObsidianVaultStatus.mockResolvedValue(status());
    const onToast = vi.fn();
    renderWithProviders(<ObsidianVaultSection contentRootAbs={ROOT} onToast={onToast} />);

    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));
    const openAnyway = await screen.findByTestId('obsidian-open-anyway');
    fireEvent.click(openAnyway);

    await waitFor(() => expect(openUrl).toHaveBeenCalledWith(DEEP_LINK));
  });

  it('"Reveal Folder" in the guidance panel reveals the content root', async () => {
    memoryTreeObsidianVaultStatus.mockResolvedValue(status());
    renderWithProviders(<ObsidianVaultSection contentRootAbs={ROOT} />);

    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));
    fireEvent.click(await screen.findByTestId('obsidian-reveal'));

    await waitFor(() => expect(revealWorkspacePath).toHaveBeenCalledWith('memory_tree/content'));
  });

  it('config not found → Install Obsidian opens the download page', async () => {
    memoryTreeObsidianVaultStatus.mockResolvedValue(status({ config_found: false }));
    renderWithProviders(<ObsidianVaultSection contentRootAbs={ROOT} />);

    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));
    fireEvent.click(await screen.findByTestId('obsidian-install'));

    await waitFor(() => expect(openUrl).toHaveBeenCalledWith('https://obsidian.md/download'));
  });

  it('Advanced config-dir override persists to localStorage and re-checks with it', async () => {
    memoryTreeObsidianVaultStatus.mockResolvedValue(status());
    renderWithProviders(<ObsidianVaultSection contentRootAbs={ROOT} />);

    // First click → not registered → guidance.
    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));
    fireEvent.click(await screen.findByTestId('obsidian-advanced-toggle'));

    const input = await screen.findByTestId('obsidian-config-dir-input');
    fireEvent.change(input, { target: { value: '/custom/obsidian' } });
    fireEvent.click(screen.getByTestId('obsidian-config-dir-save'));

    await waitFor(() =>
      expect(memoryTreeObsidianVaultStatus).toHaveBeenLastCalledWith('/custom/obsidian')
    );
    expect(localStorage.getItem('openhuman.obsidian.configDir')).toBe('/custom/obsidian');
  });

  it('detection failure degrades gracefully to the guidance panel', async () => {
    memoryTreeObsidianVaultStatus.mockRejectedValue(new Error('rpc down'));
    renderWithProviders(<ObsidianVaultSection contentRootAbs={ROOT} />);

    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));

    await waitFor(() => expect(screen.getByTestId('obsidian-vault-guidance')).toBeInTheDocument());
    expect(openUrl).not.toHaveBeenCalled();
  });

  // #2492: the absolute path that feeds the `obsidian://open?path=…` URL must
  // come from the shared workspace-link layer (the Rust-side resolver), not
  // from the `contentRootAbs` prop. The prop stays around for display, but
  // the deep link URL is composed with whatever the resolver returns.
  it('deep link uses the workspace-link resolver, not the contentRootAbs prop', async () => {
    const resolved = '/private/var/folders/canonical/memory_tree/content';
    resolveWorkspaceAbsolutePath.mockResolvedValue(resolved);
    memoryTreeObsidianVaultStatus.mockResolvedValue(status({ registered: true }));
    renderWithProviders(<ObsidianVaultSection contentRootAbs={ROOT} />);

    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));

    await waitFor(() =>
      expect(resolveWorkspaceAbsolutePath).toHaveBeenCalledWith('memory_tree/content')
    );
    await waitFor(() =>
      expect(openUrl).toHaveBeenCalledWith('obsidian://open?path=' + encodeURIComponent(resolved))
    );
  });

  // #2492: when the Rust-side resolver rejects (e.g. workspace path missing
  // on disk), the click must not silently no-op — it surfaces an error toast
  // and keeps the guidance panel expanded so the user has an escape hatch.
  it('resolver failure surfaces an error toast and keeps guidance expanded', async () => {
    resolveWorkspaceAbsolutePath.mockRejectedValue(new Error('workspace path does not exist'));
    memoryTreeObsidianVaultStatus.mockResolvedValue(status({ registered: true }));
    const onToast = vi.fn();
    renderWithProviders(<ObsidianVaultSection contentRootAbs={ROOT} onToast={onToast} />);

    fireEvent.click(screen.getByTestId('memory-open-in-obsidian'));

    await waitFor(() => expect(onToast).toHaveBeenCalled());
    expect(onToast.mock.calls[0][0].type).toBe('error');
    expect(openUrl).not.toHaveBeenCalled();
    expect(screen.getByTestId('obsidian-vault-guidance')).toBeInTheDocument();
  });
});
