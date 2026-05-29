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
    // The status dot title is the i18n'd label ('Error' in English) —
    // sourced from `channels.status.error` per `STATUS_I18N_KEYS`.
    expect(screen.getByTitle('Error')).toBeInTheDocument();
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
    expect(screen.getByTitle('Disconnected')).toBeInTheDocument();
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

  // -----------------------------------------------------------------------
  // Filter behaviour (the new search/filter feature)
  // -----------------------------------------------------------------------

  it('shows all servers when filter is the empty string', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter=""
      />
    );
    expect(screen.getByText('File Server')).toBeInTheDocument();
    expect(screen.getByText('DB Server')).toBeInTheDocument();
  });

  it('filters by display_name case-insensitively', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="FILE"
      />
    );
    expect(screen.getByText('File Server')).toBeInTheDocument();
    expect(screen.queryByText('DB Server')).not.toBeInTheDocument();
  });

  it('filters by qualified_name', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="db-server"
      />
    );
    // SERVER_2 has qualified_name 'acme/db-server' — matched
    expect(screen.getByText('DB Server')).toBeInTheDocument();
    expect(screen.queryByText('File Server')).not.toBeInTheDocument();
  });

  it('filters by description', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="reads files"
      />
    );
    // SERVER_1 has description 'Reads files' — matched
    expect(screen.getByText('File Server')).toBeInTheDocument();
    expect(screen.queryByText('DB Server')).not.toBeInTheDocument();
  });

  it('treats undefined description as empty (no false match) without crashing', () => {
    render(
      <InstalledServerList
        servers={[SERVER_2]} // description: undefined
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="undefined"
      />
    );
    // 'undefined' must not match the absent description literally — assertion
    // here is that the filter logic doesn't blow up and the no-match path runs.
    expect(screen.queryByText('DB Server')).not.toBeInTheDocument();
  });

  it('trims surrounding whitespace from the filter', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="   File   "
      />
    );
    expect(screen.getByText('File Server')).toBeInTheDocument();
    expect(screen.queryByText('DB Server')).not.toBeInTheDocument();
  });

  it('shows "no matches" message including the query when filter matches nothing', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="zzz-nope"
      />
    );
    expect(screen.getByText('No servers match "zzz-nope".')).toBeInTheDocument();
  });

  it('shows "X of Y servers" count via an aria-live region when filtering', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="File"
      />
    );
    // `status` is NOT a "name from content" role per WAI-ARIA, so the
    // accessible name doesn't come from text. Query by text and then
    // verify the live-region attributes on the same element.
    const status = screen.getByText('1 of 2 servers');
    expect(status).toHaveAttribute('role', 'status');
    expect(status).toHaveAttribute('aria-live', 'polite');
  });

  it('hides the count when filter is empty', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter=""
      />
    );
    expect(screen.queryByText(/of \d+ servers/)).not.toBeInTheDocument();
  });

  it('hides the count when filter is only whitespace', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="   "
      />
    );
    expect(screen.queryByText(/of \d+ servers/)).not.toBeInTheDocument();
  });

  it('keeps the original empty state (not the filtered no-match) when there are zero servers', () => {
    render(
      <InstalledServerList
        servers={[]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="anything"
      />
    );
    expect(screen.getByText('No MCP servers installed yet.')).toBeInTheDocument();
    expect(screen.queryByText(/No servers match/)).not.toBeInTheDocument();
  });

  // -----------------------------------------------------------------------
  // Keyboard navigation (ArrowUp / ArrowDown across server buttons)
  // -----------------------------------------------------------------------

  it('moves focus to the next server on ArrowDown', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    const first = screen.getByRole('button', { name: /File Server/i });
    const second = screen.getByRole('button', { name: /DB Server/i });
    first.focus();
    expect(first).toHaveFocus();
    fireEvent.keyDown(first, { key: 'ArrowDown' });
    expect(second).toHaveFocus();
  });

  it('moves focus to the previous server on ArrowUp', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    const first = screen.getByRole('button', { name: /File Server/i });
    const second = screen.getByRole('button', { name: /DB Server/i });
    second.focus();
    fireEvent.keyDown(second, { key: 'ArrowUp' });
    expect(first).toHaveFocus();
  });

  it('clamps focus at the last server on ArrowDown', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    const second = screen.getByRole('button', { name: /DB Server/i });
    second.focus();
    fireEvent.keyDown(second, { key: 'ArrowDown' });
    // No wrap-around.
    expect(second).toHaveFocus();
  });

  it('clamps focus at the first server on ArrowUp', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    const first = screen.getByRole('button', { name: /File Server/i });
    first.focus();
    fireEvent.keyDown(first, { key: 'ArrowUp' });
    expect(first).toHaveFocus();
  });

  it('does not move focus or preventDefault for unrelated keys', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
      />
    );
    const first = screen.getByRole('button', { name: /File Server/i });
    first.focus();
    const event = fireEvent.keyDown(first, { key: 'a' });
    // The listener should ignore unrelated keys; focus stays put.
    expect(first).toHaveFocus();
    // fireEvent returns false if preventDefault was called — verify it wasn't.
    expect(event).toBe(true);
  });

  it('arrow keys traverse only the visible (filtered) items', () => {
    render(
      <InstalledServerList
        servers={[SERVER_1, SERVER_2]}
        statuses={[]}
        selectedId={null}
        onSelect={() => {}}
        onBrowseCatalog={() => {}}
        filter="File"
      />
    );
    const visible = screen.getByRole('button', { name: /File Server/i });
    visible.focus();
    // Only one filtered item → ArrowDown should clamp (single visible)
    fireEvent.keyDown(visible, { key: 'ArrowDown' });
    expect(visible).toHaveFocus();
    expect(screen.queryByRole('button', { name: /DB Server/i })).not.toBeInTheDocument();
  });
});
