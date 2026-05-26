/**
 * Tests for McpInventoryPanel + its Export / Import tab children.
 *
 * Strategy: drive the whole panel via the parent's public surface
 * (servers prop, onInstallServer, onClose) so the test asserts the
 * actual user-visible behaviour rather than internal state shape.
 */
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { buildManifest, CURRENT_MANIFEST_SCHEMA, serializeManifest } from './McpInventoryManifest';
import McpInventoryPanel from './McpInventoryPanel';
import type { InstalledServer } from './types';

const SERVER_FS: InstalledServer = {
  server_id: 'srv-uuid-1',
  qualified_name: 'acme/fs-server',
  display_name: 'File Server',
  description: 'Reads files',
  command_kind: 'node',
  command: 'npx',
  args: ['-y', 'acme/fs-server'],
  env_keys: ['ROOT_DIR'],
  installed_at: 1_700_000_000,
};

const SERVER_DB: InstalledServer = {
  server_id: 'srv-uuid-2',
  qualified_name: 'acme/db-server',
  display_name: 'DB Server',
  command_kind: 'node',
  command: 'npx',
  args: ['-y', 'acme/db-server'],
  env_keys: ['DB_URL'],
  installed_at: 1_700_000_500,
};

const renderPanel = (overrides?: {
  servers?: InstalledServer[];
  onInstallServer?: (qualifiedName: string, prefillEnv: Record<string, string>) => void;
  onClose?: () => void;
}) =>
  render(
    <McpInventoryPanel
      servers={overrides?.servers ?? [SERVER_FS, SERVER_DB]}
      onInstallServer={overrides?.onInstallServer ?? (() => {})}
      onClose={overrides?.onClose ?? (() => {})}
    />
  );

describe('McpInventoryPanel — shell', () => {
  it('renders as an accessible modal dialog with the title labelling it', () => {
    renderPanel();
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(dialog).toHaveAttribute('aria-labelledby', 'mcp-inventory-panel-title');
    expect(screen.getByText('Sharable MCP Inventory')).toBeInTheDocument();
  });

  it('Esc closes via onClose', () => {
    const onClose = vi.fn();
    renderPanel({ onClose });
    act(() => {
      fireEvent.keyDown(document, { key: 'Escape' });
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('close button calls onClose', () => {
    const onClose = vi.fn();
    renderPanel({ onClose });
    fireEvent.click(screen.getByRole('button', { name: 'Close inventory panel' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('backdrop mousedown closes; click on dialog card does not', () => {
    const onClose = vi.fn();
    renderPanel({ onClose });
    const dialog = screen.getByRole('dialog');
    // Mousedown on the backdrop (== currentTarget) closes.
    fireEvent.mouseDown(dialog);
    expect(onClose).toHaveBeenCalledTimes(1);
    // Mousedown on a descendant must NOT close.
    fireEvent.mouseDown(screen.getByText('Sharable MCP Inventory'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('renders the Export tab by default and exposes both tabs', () => {
    renderPanel();
    const exportTab = screen.getByRole('tab', { name: 'Export' });
    const importTab = screen.getByRole('tab', { name: 'Import' });
    expect(exportTab).toHaveAttribute('aria-selected', 'true');
    expect(importTab).toHaveAttribute('aria-selected', 'false');
  });

  it('switches to the Import tab on click', () => {
    renderPanel();
    fireEvent.click(screen.getByRole('tab', { name: 'Import' }));
    expect(screen.getByRole('tab', { name: 'Import' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: 'Export' })).toHaveAttribute('aria-selected', 'false');
  });
});

describe('McpInventoryPanel — Export tab', () => {
  it('renders the empty state when there are no installed servers', () => {
    renderPanel({ servers: [] });
    expect(screen.getByText(/Install one from the catalog first/)).toBeInTheDocument();
  });

  it('renders the manifest JSON in the preview <pre> when servers exist', () => {
    renderPanel({ servers: [SERVER_FS] });
    const pre = screen.getByTestId('mcp-inventory-export-pre');
    expect(pre.textContent).toContain(`"$schema": "${CURRENT_MANIFEST_SCHEMA}"`);
    expect(pre.textContent).toContain('"qualified_name": "acme/fs-server"');
    expect(pre.textContent).toContain('"env_keys"');
    // The preview MUST NOT contain server_id, command, args, etc.
    expect(pre.textContent).not.toContain('"server_id"');
    expect(pre.textContent).not.toContain('"installed_at"');
    expect(pre.textContent).not.toContain('"command"');
  });

  it('renders the server count', () => {
    renderPanel({ servers: [SERVER_FS, SERVER_DB] });
    expect(screen.getByText('2 servers in this manifest')).toBeInTheDocument();
  });

  it('renders the privacy banner verbatim', () => {
    renderPanel({ servers: [SERVER_FS] });
    expect(screen.getByText('What is in this manifest')).toBeInTheDocument();
    expect(
      screen.getByText(/env-variable KEY NAMES, and non-secret config only/)
    ).toBeInTheDocument();
  });

  it('Download button creates a Blob URL, triggers a click on a hidden link, and revokes', async () => {
    const createObjectURL = vi.fn().mockReturnValue('blob:fake-url-123');
    const revokeObjectURL = vi.fn();
    const originalCreate = URL.createObjectURL;
    const originalRevoke = URL.revokeObjectURL;
    URL.createObjectURL = createObjectURL;
    URL.revokeObjectURL = revokeObjectURL;
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    try {
      renderPanel({ servers: [SERVER_FS] });
      fireEvent.click(screen.getByRole('button', { name: /Download the manifest/ }));
      expect(createObjectURL).toHaveBeenCalledTimes(1);
      const blobArg = createObjectURL.mock.calls[0][0] as Blob;
      expect(blobArg.type).toBe('application/json');
      expect(clickSpy).toHaveBeenCalledTimes(1);
      // handleDownload defers revokeObjectURL via setTimeout(..., 0); flush.
      await new Promise(resolve => setTimeout(resolve, 0));
      expect(revokeObjectURL).toHaveBeenCalledWith('blob:fake-url-123');
    } finally {
      URL.createObjectURL = originalCreate;
      URL.revokeObjectURL = originalRevoke;
      clickSpy.mockRestore();
    }
  });

  it('Copy button writes the serialized manifest to the clipboard', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = (navigator as { clipboard?: unknown }).clipboard;
    Object.defineProperty(navigator, 'clipboard', {
      writable: true,
      configurable: true,
      value: { writeText },
    });
    try {
      renderPanel({ servers: [SERVER_FS] });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: /Copy the manifest JSON/ }));
      });
      expect(writeText).toHaveBeenCalledTimes(1);
      const arg = writeText.mock.calls[0][0] as string;
      expect(arg).toContain(`"$schema": "${CURRENT_MANIFEST_SCHEMA}"`);
      expect(arg).toContain('"qualified_name": "acme/fs-server"');
    } finally {
      if (originalClipboard === undefined) {
        delete (navigator as { clipboard?: unknown }).clipboard;
      } else {
        Object.defineProperty(navigator, 'clipboard', {
          writable: true,
          configurable: true,
          value: originalClipboard,
        });
      }
    }
  });
});

describe('McpInventoryPanel — Import tab', () => {
  beforeEach(() => {
    // Each test starts on the Export tab by default. Most Import-tab
    // tests want to jump straight to Import; the per-test setup does so.
  });

  const switchToImport = () => {
    fireEvent.click(screen.getByRole('tab', { name: 'Import' }));
  };

  it('renders the trust banner', () => {
    renderPanel();
    switchToImport();
    expect(screen.getByText('Treat imported manifests as untrusted code')).toBeInTheDocument();
  });

  it('Preview button is disabled until the textarea has content', () => {
    renderPanel();
    switchToImport();
    expect(screen.getByRole('button', { name: 'Preview' })).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), { target: { value: '{}' } });
    expect(screen.getByRole('button', { name: 'Preview' })).not.toBeDisabled();
  });

  it('shows a role=alert with the parse-error prefix when the JSON is invalid', () => {
    renderPanel();
    switchToImport();
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), {
      target: { value: '{not valid' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toMatch(/Could not parse manifest:/);
  });

  it('shows a role=alert when an unknown $schema is presented', () => {
    renderPanel();
    switchToImport();
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), {
      target: {
        value: JSON.stringify({
          $schema: 'wrong',
          exported_at: '2026-05-25T00:00:00Z',
          exported_by: 'x',
          servers: [],
        }),
      },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    expect(screen.getByRole('alert').textContent).toMatch(/Unsupported manifest schema/);
  });

  it('SECURITY: refuses a manifest that smuggles an env value map and surfaces a clear alert', () => {
    renderPanel();
    switchToImport();
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), {
      target: {
        value: JSON.stringify({
          $schema: CURRENT_MANIFEST_SCHEMA,
          exported_at: '2026-05-25T00:00:00Z',
          exported_by: 'attacker',
          servers: [
            {
              qualified_name: 'evil/server',
              display_name: 'Evil',
              env_keys: ['SECRET'],
              env: { SECRET: 'attacker-value' },
            },
          ],
        }),
      },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    expect(screen.getByRole('alert').textContent).toMatch(/secret values/i);
    // Critically: no Install button should have been rendered for the rejected entry.
    expect(screen.queryByRole('button', { name: /Install Evil/ })).not.toBeInTheDocument();
  });

  it('previews a valid manifest with a per-entry status row', () => {
    const valid = serializeManifest(buildManifest([SERVER_FS, SERVER_DB]));
    renderPanel({ servers: [SERVER_FS] }); // SERVER_FS already installed; SERVER_DB is new
    switchToImport();
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), { target: { value: valid } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));

    // Heading + counts live region
    expect(screen.getByRole('heading', { name: 'Preview' })).toBeInTheDocument();
    const status = screen.getByRole('status', { hidden: true });
    // The summary should mention 2 total — 1 new, 1 already installed.
    expect(status.textContent).toMatch(/2 servers/);
    expect(status.textContent).toMatch(/1 new/);
    expect(status.textContent).toMatch(/1 already installed/);

    // SERVER_DB row has an Install button; SERVER_FS row is skipped.
    expect(
      screen.getByRole('button', { name: 'Install DB Server from this manifest' })
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Install File Server/ })).not.toBeInTheDocument();
  });

  it('clicking Install hands the qualified_name + empty env prefill to onInstallServer', () => {
    const onInstallServer = vi.fn();
    const valid = serializeManifest(buildManifest([SERVER_DB]));
    renderPanel({ servers: [], onInstallServer });
    switchToImport();
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), { target: { value: valid } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));

    fireEvent.click(screen.getByRole('button', { name: 'Install DB Server from this manifest' }));
    expect(onInstallServer).toHaveBeenCalledTimes(1);
    expect(onInstallServer).toHaveBeenCalledWith('acme/db-server', { DB_URL: '' });
  });

  it('clicking Install also closes the panel (handoff to the existing InstallDialog flow)', () => {
    const onClose = vi.fn();
    const valid = serializeManifest(buildManifest([SERVER_DB]));
    renderPanel({ servers: [], onClose });
    switchToImport();
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), { target: { value: valid } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    fireEvent.click(screen.getByRole('button', { name: 'Install DB Server from this manifest' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('Clear button resets the textarea, preview, and any parse error', () => {
    renderPanel();
    switchToImport();
    const textarea = screen.getByLabelText('Paste manifest JSON');
    fireEvent.change(textarea, { target: { value: '{bad' } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    expect(screen.getByRole('alert')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
    expect(textarea).toHaveValue('');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('typing in the textarea live-clears a stale parse error', () => {
    renderPanel();
    switchToImport();
    const textarea = screen.getByLabelText('Paste manifest JSON');
    fireEvent.change(textarea, { target: { value: '{bad' } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    expect(screen.getByRole('alert')).toBeInTheDocument();
    fireEvent.change(textarea, { target: { value: '{better-typing' } });
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('the empty-manifest case shows a helpful message and no Install buttons', () => {
    const emptyManifest = serializeManifest(buildManifest([]));
    renderPanel();
    switchToImport();
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), {
      target: { value: emptyManifest },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    expect(screen.getByText('Manifest contains no servers.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Install/ })).not.toBeInTheDocument();
  });

  it('shows env_keys for each preview row when present', () => {
    const valid = serializeManifest(buildManifest([SERVER_FS]));
    renderPanel({ servers: [] });
    switchToImport();
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), { target: { value: valid } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    // The env_keys for SERVER_FS is just ['ROOT_DIR']
    const previewSection = screen.getByRole('heading', { name: 'Preview' }).closest('section')!;
    expect(within(previewSection).getByText(/ROOT_DIR/)).toBeInTheDocument();
  });

  // The file-upload paths are tested via a controllable FileReader stub so
  // we can drive onload/onerror deterministically; jsdom's built-in
  // FileReader works on File but doesn't fire events synchronously in a
  // way that aligns with the synchronous fireEvent/expect cycle here.
  it('uploading a valid JSON file populates the textarea and previews it', async () => {
    const valid = serializeManifest(buildManifest([SERVER_DB]));
    const OriginalFileReader = window.FileReader;
    class StubReader {
      onload: ((this: FileReader, ev: ProgressEvent<FileReader>) => unknown) | null = null;
      onerror: ((this: FileReader, ev: ProgressEvent<FileReader>) => unknown) | null = null;
      result: string | ArrayBuffer | null = null;
      readAsText(_: Blob) {
        this.result = valid;
        if (this.onload)
          this.onload.call(this as unknown as FileReader, {} as ProgressEvent<FileReader>);
      }
    }
    (window as unknown as { FileReader: unknown }).FileReader = StubReader;
    try {
      renderPanel({ servers: [] });
      switchToImport();
      const fileInput = screen.getByLabelText('Upload a manifest .json file') as HTMLInputElement;
      const file = new File([valid], 'manifest.json', { type: 'application/json' });
      await act(async () => {
        fireEvent.change(fileInput, { target: { files: [file] } });
      });
      expect(screen.getByLabelText('Paste manifest JSON')).toHaveValue(valid);
      // Preview was rendered automatically by the onload's parseManifest call.
      expect(screen.getByRole('heading', { name: 'Preview' })).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: 'Install DB Server from this manifest' })
      ).toBeInTheDocument();
    } finally {
      (window as unknown as { FileReader: unknown }).FileReader = OriginalFileReader;
    }
  });

  it('uploading a file larger than 1 MB is refused with a file-error alert and clears any prior preview', () => {
    renderPanel({ servers: [] });
    switchToImport();
    // Stage a valid preview first so we can assert that the refused upload
    // clears it (per the stale-state-clearing contract in handleFileChange).
    const valid = serializeManifest(buildManifest([SERVER_DB]));
    fireEvent.change(screen.getByLabelText('Paste manifest JSON'), { target: { value: valid } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
    expect(screen.getByRole('heading', { name: 'Preview' })).toBeInTheDocument();

    const fileInput = screen.getByLabelText('Upload a manifest .json file') as HTMLInputElement;
    // 1.5 MB sparse File — `size` is what handleFileChange gates on.
    const big = new File([new Uint8Array(1_500_000)], 'big.json', { type: 'application/json' });
    fireEvent.change(fileInput, { target: { files: [big] } });
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toMatch(/too large/i);
    // Stale preview is gone.
    expect(screen.queryByRole('heading', { name: 'Preview' })).not.toBeInTheDocument();
  });

  it('FileReader onerror surfaces a file-read-failed alert and clears any prior preview', async () => {
    const OriginalFileReader = window.FileReader;
    class FailingReader {
      onload: ((this: FileReader, ev: ProgressEvent<FileReader>) => unknown) | null = null;
      onerror: ((this: FileReader, ev: ProgressEvent<FileReader>) => unknown) | null = null;
      readAsText(_: Blob) {
        if (this.onerror)
          this.onerror.call(this as unknown as FileReader, {} as ProgressEvent<FileReader>);
      }
    }
    (window as unknown as { FileReader: unknown }).FileReader = FailingReader;
    try {
      renderPanel({ servers: [] });
      switchToImport();
      // Stage a valid preview to confirm it's wiped on read failure.
      const valid = serializeManifest(buildManifest([SERVER_DB]));
      fireEvent.change(screen.getByLabelText('Paste manifest JSON'), { target: { value: valid } });
      fireEvent.click(screen.getByRole('button', { name: 'Preview' }));
      expect(screen.getByRole('heading', { name: 'Preview' })).toBeInTheDocument();

      const fileInput = screen.getByLabelText('Upload a manifest .json file') as HTMLInputElement;
      const file = new File(['anything'], 'broken.json', { type: 'application/json' });
      await act(async () => {
        fireEvent.change(fileInput, { target: { files: [file] } });
      });
      const alert = screen.getByRole('alert');
      expect(alert.textContent).toMatch(/Could not read file/i);
      expect(screen.queryByRole('heading', { name: 'Preview' })).not.toBeInTheDocument();
    } finally {
      (window as unknown as { FileReader: unknown }).FileReader = OriginalFileReader;
    }
  });
});
