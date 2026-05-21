/**
 * Tests for InstalledServerList — covers empty state, server list rendering,
 * status dot classes, tool count display, selection highlight, and callbacks.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import InstalledServerList from './InstalledServerList';
import type { ConnStatus, InstalledServer } from './types';

const SERVER_A: InstalledServer = {
  server_id: 'srv-a',
  qualified_name: 'acme/server-a',
  display_name: 'Server A',
  command_kind: 'node',
  command: 'node',
  args: [],
  env_keys: [],
  installed_at: 0,
};

const SERVER_B: InstalledServer = {
  server_id: 'srv-b',
  qualified_name: 'acme/server-b',
  display_name: 'Server B',
  command_kind: 'node',
  command: 'node',
  args: [],
  env_keys: [],
  installed_at: 0,
};

describe('InstalledServerList', () => {
  it('renders header and Browse catalog button', () => {
    const onBrowse = vi.fn();
    render(
      <InstalledServerList
        servers={[]}
        statuses={[]}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={onBrowse}
      />
    );
    expect(screen.getByText('Installed')).toBeInTheDocument();
    // Header browse catalog link
    expect(screen.getAllByRole('button', { name: /browse catalog/i }).length).toBeGreaterThan(0);
  });

  it('shows empty-state prompt and Browse catalog CTA when no servers installed', () => {
    const onBrowse = vi.fn();
    render(
      <InstalledServerList
        servers={[]}
        statuses={[]}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={onBrowse}
      />
    );
    expect(screen.getByText(/no mcp servers installed yet/i)).toBeInTheDocument();
    // At least two browse catalog buttons (header + CTA)
    const buttons = screen.getAllByRole('button', { name: /browse catalog/i });
    expect(buttons.length).toBeGreaterThanOrEqual(2);

    fireEvent.click(buttons[buttons.length - 1]); // CTA button
    expect(onBrowse).toHaveBeenCalled();
  });

  it('calls onBrowseCatalog when header button is clicked', () => {
    const onBrowse = vi.fn();
    render(
      <InstalledServerList
        servers={[]}
        statuses={[]}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={onBrowse}
      />
    );
    fireEvent.click(screen.getAllByRole('button', { name: /browse catalog/i })[0]);
    expect(onBrowse).toHaveBeenCalledTimes(1);
  });

  it('renders server display names', () => {
    render(
      <InstalledServerList
        servers={[SERVER_A, SERVER_B]}
        statuses={[]}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={vi.fn()}
      />
    );
    expect(screen.getByText('Server A')).toBeInTheDocument();
    expect(screen.getByText('Server B')).toBeInTheDocument();
  });

  it('calls onSelect with the correct server_id when a server button is clicked', () => {
    const onSelect = vi.fn();
    render(
      <InstalledServerList
        servers={[SERVER_A, SERVER_B]}
        statuses={[]}
        selectedId={null}
        onSelect={onSelect}
        onBrowseCatalog={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /server b/i }));
    expect(onSelect).toHaveBeenCalledWith('srv-b');
  });

  it('applies selected styling to the active server', () => {
    render(
      <InstalledServerList
        servers={[SERVER_A, SERVER_B]}
        statuses={[]}
        selectedId="srv-a"
        onSelect={vi.fn()}
        onBrowseCatalog={vi.fn()}
      />
    );
    const serverABtn = screen.getByRole('button', { name: /server a/i });
    // Selected button should have primary-50 background class
    expect(serverABtn.className).toContain('bg-primary-50');
    // Non-selected should not
    const serverBBtn = screen.getByRole('button', { name: /server b/i });
    expect(serverBBtn.className).not.toContain('bg-primary-50');
  });

  it('shows tool count when server is connected with tools', () => {
    const statuses: ConnStatus[] = [
      {
        server_id: 'srv-a',
        qualified_name: 'acme/server-a',
        display_name: 'Server A',
        status: 'connected',
        tool_count: 5,
      },
    ];
    render(
      <InstalledServerList
        servers={[SERVER_A]}
        statuses={statuses}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={vi.fn()}
      />
    );
    expect(screen.getByText('5 tools')).toBeInTheDocument();
  });

  it('shows singular "tool" when tool count is 1', () => {
    const statuses: ConnStatus[] = [
      {
        server_id: 'srv-a',
        qualified_name: 'acme/server-a',
        display_name: 'Server A',
        status: 'connected',
        tool_count: 1,
      },
    ];
    render(
      <InstalledServerList
        servers={[SERVER_A]}
        statuses={statuses}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={vi.fn()}
      />
    );
    expect(screen.getByText('1 tool')).toBeInTheDocument();
  });

  it('does not show tool count when server is disconnected', () => {
    const statuses: ConnStatus[] = [
      {
        server_id: 'srv-a',
        qualified_name: 'acme/server-a',
        display_name: 'Server A',
        status: 'disconnected',
        tool_count: 5,
      },
    ];
    render(
      <InstalledServerList
        servers={[SERVER_A]}
        statuses={statuses}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={vi.fn()}
      />
    );
    expect(screen.queryByText(/5 tool/)).not.toBeInTheDocument();
  });

  it('does not show tool count when connected but tool_count is 0', () => {
    const statuses: ConnStatus[] = [
      {
        server_id: 'srv-a',
        qualified_name: 'acme/server-a',
        display_name: 'Server A',
        status: 'connected',
        tool_count: 0,
      },
    ];
    render(
      <InstalledServerList
        servers={[SERVER_A]}
        statuses={statuses}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={vi.fn()}
      />
    );
    expect(screen.queryByText(/tool/)).not.toBeInTheDocument();
  });

  it('defaults status to disconnected when no connStatus present', () => {
    // With no statuses, the status dot should use the disconnected colour class.
    const { container } = render(
      <InstalledServerList
        servers={[SERVER_A]}
        statuses={[]}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={vi.fn()}
      />
    );
    const dot = container.querySelector('[title="disconnected"]');
    expect(dot).not.toBeNull();
    expect(dot!.className).toContain('bg-stone-300');
  });

  it('renders error status dot for error status', () => {
    const statuses: ConnStatus[] = [
      {
        server_id: 'srv-a',
        qualified_name: 'acme/server-a',
        display_name: 'Server A',
        status: 'error',
        tool_count: 0,
      },
    ];
    const { container } = render(
      <InstalledServerList
        servers={[SERVER_A]}
        statuses={statuses}
        selectedId={null}
        onSelect={vi.fn()}
        onBrowseCatalog={vi.fn()}
      />
    );
    const dot = container.querySelector('[title="error"]');
    expect(dot).not.toBeNull();
    expect(dot!.className).toContain('bg-coral-500');
  });
});
