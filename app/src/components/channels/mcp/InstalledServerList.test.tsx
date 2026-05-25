/**
 * Tests for InstalledServerList — static rendering component.
 * No async behavior; all branches covered synchronously.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import InstalledServerList from './InstalledServerList';
import type { ConnStatus, InstalledServer } from './types';

const SERVER_1: InstalledServer = {
  server_id: 'srv-1',
  qualified_name: 'acme/fs-server',
  display_name: 'File Server',
  description: 'Reads files',
  command_kind: 'node',
  command: 'npx',
  args: ['-y', 'acme/fs-server'],
  env_keys: [],
  installed_at: 1_700_000_000,
};

const SERVER_2: InstalledServer = {
  server_id: 'srv-2',
  qualified_name: 'acme/db-server',
  display_name: 'DB Server',
  description: undefined,
  command_kind: 'node',
  command: 'npx',
  args: ['-y', 'acme/db-server'],
  env_keys: ['DB_URL'],
  installed_at: 1_700_000_001,
};

const STATUS_CONNECTED: ConnStatus = {
  server_id: 'srv-1',
  qualified_name: 'acme/fs-server',
  display_name: 'File Server',
  status: 'connected',
  tool_count: 3,
};

const STATUS_ERROR: ConnStatus = {
  server_id: 'srv-2',
  qualified_name: 'acme/db-server',
  display_name: 'DB Server',
  status: 'error',
  tool_count: 0,
  last_error: 'Connection refused',
};

describe('InstalledServerList', () => {
  it('shows empty state with Browse catalog button when no servers', () => {
    const onBrowse = vi.fn();
    render(
      <InstalledServerList
        servers={[]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={onBrowse}
      />
    );
    expect(screen.getByText('No MCP servers installed yet.')).toBeInTheDocument();
    // Two "Browse catalog" buttons exist: header link and empty-state CTA.
    // Click the CTA (second one) to verify the prop fires.
    const btns = screen.getAllByRole('button', { name: 'Browse catalog' });
    expect(btns).toHaveLength(2);
    fireEvent.click(btns[1]);
    expect(onBrowse).toHaveBeenCalledTimes(1);
  });

  it('renders all server display names', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    expect(screen.getByText('File Server')).toBeInTheDocument();
    expect(screen.getByText('DB Server')).toBeInTheDocument();
  });

  it('calls onSelect with the correct server_id when clicked', () => {
    const onSelect = vi.fn();
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={onSelect}
        onBrowseCatalog={() => {}}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /File Server/i }));
    expect(onSelect).toHaveBeenCalledWith('srv-1');
  });

  it('applies selected styling to the active server', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId="srv-1"
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    const btn = screen.getByRole('button', { name: /File Server/i });
    expect(btn.className).toMatch(/border-primary/);
  });

  it('shows tool count when connected with tools', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1]}
        statuses={[STATUS_CONNECTED]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    expect(screen.getByText('3 tools')).toBeInTheDocument();
  });

  it('does not show tool count when disconnected', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1]}
        statuses={[
          {
            server_id: 'srv-1',
            qualified_name: 'acme/fs-server',
            display_name: 'File Server',
            status: 'disconnected',
            tool_count: 0,
          },
        ]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    expect(screen.queryByText(/tools/)).not.toBeInTheDocument();
  });

  it('does not show tool count when connected but tool_count is 0', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1]}
        statuses={[{ ...STATUS_CONNECTED, tool_count: 0 }]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    expect(screen.queryByText(/tools/)).not.toBeInTheDocument();
  });

  it('shows singular "tool" when tool count is 1', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1]}
        statuses={[{ ...STATUS_CONNECTED, tool_count: 1 }]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    expect(screen.getByText('1 tool')).toBeInTheDocument();
  });

  it('applies error status dot to error server', () => {
    render(
      <InstalledServerList
        servers={[SERVER_2]}
        statuses={[STATUS_ERROR]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    // The status dot has title="error"
    expect(screen.getByTitle('error')).toBeInTheDocument();
  });

  it('falls back to disconnected status when no matching status entry', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    expect(screen.getByTitle('disconnected')).toBeInTheDocument();
  });

  // -----------------------------------------------------------------------
  // Defensive rendering with malformed props
  // -----------------------------------------------------------------------

  it('does not crash when statuses is undefined', () => {
    // Guard: passing undefined instead of [] should not throw
    render(
      <InstalledServerList
        servers={[SERVER_1]}
        statuses={undefined as unknown as ConnStatus[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    // Server name still renders; status falls back to disconnected
    expect(screen.getByText('File Server')).toBeInTheDocument();
  });

  it('calls onBrowseCatalog from the header link', () => {
    const onBrowse = vi.fn();
    render(
      <InstalledServerList
        servers={[SERVER_1]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={onBrowse}
      />
    );
    // Only the header link button is present when servers are non-empty.
    fireEvent.click(screen.getByRole('button', { name: 'Browse catalog' }));
    expect(onBrowse).toHaveBeenCalledTimes(1);
  });
});
