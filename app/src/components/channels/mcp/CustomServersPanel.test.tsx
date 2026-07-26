/**
 * Tests for CustomServersPanel — the hand-added-server pane on the MCP
 * Servers tab.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import CustomServersPanel from './CustomServersPanel';
import type { ConnStatus, InstalledServer } from './types';

const mockAddCustom = vi.fn();
const mockUpdateCustom = vi.fn();
const mockUninstall = vi.fn();
const mockConnect = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    addCustom: (...args: unknown[]) => mockAddCustom(...args),
    updateCustom: (...args: unknown[]) => mockUpdateCustom(...args),
    uninstall: (...args: unknown[]) => mockUninstall(...args),
    connect: (...args: unknown[]) => mockConnect(...args),
  },
}));

const server = (over: Partial<InstalledServer> = {}): InstalledServer => ({
  server_id: 'srv-1',
  qualified_name: 'custom/my-server',
  display_name: 'My Server',
  command_kind: 'node',
  command: 'npx',
  args: [],
  env_keys: [],
  installed_at: 0,
  enabled: true,
  provenance: 'custom',
  transport: { kind: 'stdio' },
  ...over,
});

const statusFor = (server_id: string): ConnStatus => ({
  server_id,
  qualified_name: 'custom/my-server',
  display_name: 'My Server',
  status: 'connected',
  tool_count: 2,
});

const renderPanel = (servers: InstalledServer[], statuses: ConnStatus[] = []) => {
  const onChanged = vi.fn().mockResolvedValue(true);
  const onSelectServer = vi.fn();
  render(
    <CustomServersPanel
      servers={servers}
      statuses={statuses}
      onChanged={onChanged}
      onSelectServer={onSelectServer}
    />
  );
  return { onChanged, onSelectServer };
};

describe('CustomServersPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockAddCustom.mockResolvedValue(server());
    mockConnect.mockResolvedValue({ server_id: 'srv-1', status: 'connected', tools: [] });
    mockUninstall.mockResolvedValue({ server_id: 'srv-1', removed: true });
    mockUpdateCustom.mockResolvedValue(server());
  });

  it('shows the empty state when there are no custom servers', () => {
    renderPanel([]);
    expect(screen.getByText(/No custom servers yet/)).toBeInTheDocument();
  });

  /** The panel shares the tab's `servers` array, which also holds catalog
   *  installs — those belong in the table, not here. */
  it('lists only custom servers, never registry installs', () => {
    renderPanel([
      server(),
      server({
        server_id: 'srv-2',
        display_name: 'Registry Server',
        provenance: 'registry',
        qualified_name: 'acme/registry-server',
      }),
    ]);
    expect(screen.getByText('My Server')).toBeInTheDocument();
    expect(screen.queryByText('Registry Server')).not.toBeInTheDocument();
  });

  /** A record from a core predating the column has no `provenance`; it must be
   *  read as a registry install, not surface here as editable. */
  it('treats a record with no provenance as a registry install', () => {
    renderPanel([server({ provenance: undefined, display_name: 'Legacy Server' })]);
    expect(screen.queryByText('Legacy Server')).not.toBeInTheDocument();
    expect(screen.getByText(/No custom servers yet/)).toBeInTheDocument();
  });

  it('renders the live status and transport of each custom server', () => {
    renderPanel([server()], [statusFor('srv-1')]);
    expect(screen.getByText('Local')).toBeInTheDocument();
    // The status must come from the `statuses` prop, not the default: without
    // this the whole status wiring could be dead and the test would still pass.
    expect(screen.getByText('Connected')).toBeInTheDocument();
  });

  it('falls back to disconnected when the server has no status entry', () => {
    renderPanel([server()], []);
    expect(screen.queryByText('Connected')).not.toBeInTheDocument();
  });

  it('labels an http_remote server as remote', () => {
    renderPanel([server({ transport: { kind: 'http_remote', url: 'https://x.io/mcp' } })]);
    expect(screen.getByText('Remote')).toBeInTheDocument();
  });

  it('opens the detail view when a server row is clicked', () => {
    const { onSelectServer } = renderPanel([server()]);
    // The row's own name; the Edit/Remove buttons carry it only in aria-label.
    fireEvent.click(screen.getByText('My Server'));
    expect(onSelectServer).toHaveBeenCalledWith('srv-1');
  });

  it('removes a server and refreshes, once confirmed', async () => {
    const { onChanged } = renderPanel([server()]);
    fireEvent.click(screen.getByRole('button', { name: 'Remove My Server' }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(mockUninstall).toHaveBeenCalledWith('srv-1'));
    expect(onChanged).toHaveBeenCalled();
  });

  // Removal permanently drops the server's stored env values and OAuth bundle —
  // secrets the user typed by hand and cannot recover — so a single misclick on
  // a row button must not be enough to destroy them.
  it('asks before removing rather than deleting on the first click', () => {
    renderPanel([server()]);
    fireEvent.click(screen.getByRole('button', { name: 'Remove My Server' }));
    expect(mockUninstall).not.toHaveBeenCalled();
    const dialog = screen.getByTestId('mcp-custom-remove-confirm');
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(dialog).toHaveTextContent('Remove My Server?');
  });

  it('leaves the server installed when the confirmation is cancelled', () => {
    renderPanel([server()]);
    fireEvent.click(screen.getByRole('button', { name: 'Remove My Server' }));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByTestId('mcp-custom-remove-confirm')).toBeNull();
    expect(mockUninstall).not.toHaveBeenCalled();
  });

  it('dismisses the confirmation on Escape without removing', () => {
    renderPanel([server()]);
    fireEvent.click(screen.getByRole('button', { name: 'Remove My Server' }));
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByTestId('mcp-custom-remove-confirm')).toBeNull();
    expect(mockUninstall).not.toHaveBeenCalled();
  });

  it('surfaces a remove failure instead of silently doing nothing', async () => {
    mockUninstall.mockRejectedValue(new Error('server is busy'));
    renderPanel([server()]);
    fireEvent.click(screen.getByRole('button', { name: 'Remove My Server' }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('server is busy');
  });

  it('adds a server through the form and dials it', async () => {
    const { onChanged } = renderPanel([]);
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));

    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'New Server' } });
    fireEvent.change(screen.getByLabelText('Command'), { target: { value: 'npx' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));

    await waitFor(() => expect(mockAddCustom).toHaveBeenCalled());
    expect(mockAddCustom.mock.calls[0][0]).toMatchObject({
      display_name: 'New Server',
      transport: 'stdio',
      command: 'npx',
    });
    // The row is dialed through the ordinary connect path.
    await waitFor(() => expect(mockConnect).toHaveBeenCalledWith('srv-1'));
    expect(onChanged).toHaveBeenCalled();
  });

  /** A server whose command is wrong still exists and is editable — the add
   *  must not be reported as failed just because the first dial didn't land. */
  it('keeps an added server when the first connect fails', async () => {
    mockConnect.mockRejectedValue(new Error('ENOENT'));
    const { onChanged } = renderPanel([]);
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Broken' } });
    fireEvent.change(screen.getByLabelText('Command'), { target: { value: 'nope' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));

    await waitFor(() => expect(mockAddCustom).toHaveBeenCalled());
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    // The form closed: the add itself succeeded.
    await waitFor(() => expect(screen.queryByTestId('custom-server-form')).not.toBeInTheDocument());
  });

  it('edits an existing server', async () => {
    renderPanel([server()]);
    fireEvent.click(screen.getByRole('button', { name: 'Edit My Server' }));
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Renamed' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => expect(mockUpdateCustom).toHaveBeenCalled());
    expect(mockUpdateCustom.mock.calls[0][0]).toMatchObject({
      server_id: 'srv-1',
      display_name: 'Renamed',
    });
  });
});
