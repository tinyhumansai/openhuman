/**
 * Tests for the AddMemorySourceDialog — focused on the Composio connection
 * picker: deduplication, readable labels, and no raw connection IDs in the
 * rendered dropdown (issue #3356).
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { listConnections } from '../../lib/composio/composioApi';
import { renderWithProviders } from '../../test/test-utils';
import { AddMemorySourceDialog, deduplicateConnections } from './AddMemorySourceDialog';

// ---------------------------------------------------------------------------
// Module mocks
// ---------------------------------------------------------------------------

vi.mock('../../lib/composio/composioApi', () => ({ listConnections: vi.fn() }));

vi.mock('../../services/memorySourcesService', () => ({
  addMemorySource: vi.fn(),
  SOURCE_KIND_ICONS: {
    folder: '📁',
    composio: '🔗',
    github_repo: '🐙',
    rss_feed: '📡',
    web_page: '🌐',
    twitter_query: '🐦',
  },
  SOURCE_KIND_LABEL_KEYS: {
    folder: 'memorySources.kind.folder',
    composio: 'memorySources.kind.composio',
    github_repo: 'memorySources.kind.github_repo',
    rss_feed: 'memorySources.kind.rss_feed',
    web_page: 'memorySources.kind.web_page',
    twitter_query: 'memorySources.kind.twitter_query',
  },
}));

const mockListConnections = listConnections as ReturnType<typeof vi.fn>;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

function renderDialog() {
  const onClose = vi.fn();
  const onAdded = vi.fn();
  renderWithProviders(<AddMemorySourceDialog open onClose={onClose} onAdded={onAdded} />);
  return { onClose, onAdded };
}

async function openComposioStep() {
  renderDialog();
  // The i18n context renders the real English string from en.ts
  const integrationBtn = screen.getByText('Integration');
  fireEvent.click(integrationBtn);
  // Wait for async connection fetch
  await waitFor(() => expect(mockListConnections).toHaveBeenCalledTimes(1));
}

// ---------------------------------------------------------------------------
// Unit tests: deduplicateConnections helper
// ---------------------------------------------------------------------------

describe('deduplicateConnections', () => {
  it('returns an empty array for empty input', () => {
    expect(deduplicateConnections([], 'Account')).toEqual([]);
  });

  it('uses accountEmail as the identity label', () => {
    const conn = {
      id: 'conn-1',
      toolkit: 'Gmail',
      status: 'ACTIVE',
      accountEmail: 'user@example.com',
    };
    const result = deduplicateConnections([conn], 'Account');
    expect(result).toHaveLength(1);
    expect(result[0].label).toBe('Gmail · user@example.com');
    expect(result[0].conn.id).toBe('conn-1');
  });

  it('falls back to workspace when accountEmail is absent', () => {
    const conn = { id: 'conn-2', toolkit: 'Slack', status: 'ACTIVE', workspace: 'my-workspace' };
    const result = deduplicateConnections([conn], 'Account');
    expect(result[0].label).toBe('Slack · my-workspace');
  });

  it('falls back to username when neither email nor workspace is present', () => {
    const conn = { id: 'conn-3', toolkit: 'GitHub', status: 'ACTIVE', username: 'octocat' };
    const result = deduplicateConnections([conn], 'Account');
    expect(result[0].label).toBe('GitHub · octocat');
  });

  it('uses numbered Account label when no identity field is available', () => {
    const conn = { id: 'conn-x', toolkit: 'Notion', status: 'ACTIVE' };
    const result = deduplicateConnections([conn], 'Account');
    expect(result[0].label).toBe('Notion · Account 1');
  });

  it('numbers multiple no-identity connections per toolkit', () => {
    const conns = [
      { id: 'conn-a', toolkit: 'Notion', status: 'ACTIVE' },
      { id: 'conn-b', toolkit: 'Notion', status: 'ACTIVE' },
    ];
    const result = deduplicateConnections(conns, 'Account');
    expect(result).toHaveLength(2);
    expect(result[0].label).toBe('Notion · Account 1');
    expect(result[1].label).toBe('Notion · Account 2');
  });

  it('deduplicates connections with the same toolkit and identity', () => {
    const conns = [
      { id: 'conn-1', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'a@example.com' },
      { id: 'conn-2', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'a@example.com' },
    ];
    const result = deduplicateConnections(conns, 'Account');
    expect(result).toHaveLength(1);
    expect(result[0].conn.id).toBe('conn-1');
    expect(result[0].label).toBe('Gmail · a@example.com');
  });

  it('keeps connections with the same toolkit but different identities', () => {
    const conns = [
      { id: 'conn-1', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'a@example.com' },
      { id: 'conn-2', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'b@example.com' },
    ];
    const result = deduplicateConnections(conns, 'Account');
    expect(result).toHaveLength(2);
  });

  it('does not expose raw connection IDs in any label', () => {
    const conns = [
      { id: 'raw-uuid-abc123', toolkit: 'Linear', status: 'ACTIVE' },
      { id: 'raw-uuid-def456', toolkit: 'Linear', status: 'ACTIVE' },
    ];
    const result = deduplicateConnections(conns, 'Account');
    for (const { label } of result) {
      expect(label).not.toContain('raw-uuid');
    }
  });

  it('numbers no-identity connections per toolkit independently', () => {
    const conns = [
      { id: 'n-1', toolkit: 'Notion', status: 'ACTIVE' },
      { id: 's-1', toolkit: 'Slack', status: 'ACTIVE' },
      { id: 'n-2', toolkit: 'Notion', status: 'ACTIVE' },
    ];
    const result = deduplicateConnections(conns, 'Account');
    expect(result).toHaveLength(3);
    expect(result.find(r => r.conn.id === 'n-1')?.label).toBe('Notion · Account 1');
    expect(result.find(r => r.conn.id === 'n-2')?.label).toBe('Notion · Account 2');
    expect(result.find(r => r.conn.id === 's-1')?.label).toBe('Slack · Account 1');
  });

  it('prefers ACTIVE over EXPIRED when deduplicating same toolkit+identity', () => {
    // Backend returns EXPIRED first — the ACTIVE one should win
    const conns = [
      { id: 'conn-expired', toolkit: 'Gmail', status: 'EXPIRED', accountEmail: 'x@example.com' },
      { id: 'conn-active', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'x@example.com' },
    ];
    const result = deduplicateConnections(conns, 'Account');
    expect(result).toHaveLength(1);
    expect(result[0].conn.id).toBe('conn-active');
  });

  it('deduplicates identity-less connections with the same conn.id', () => {
    // Same connection returned twice with no identity — must not produce duplicate React keys
    const conns = [
      { id: 'conn-same', toolkit: 'Notion', status: 'ACTIVE' },
      { id: 'conn-same', toolkit: 'Notion', status: 'ACTIVE' },
    ];
    const result = deduplicateConnections(conns, 'Account');
    expect(result).toHaveLength(1);
    expect(result[0].conn.id).toBe('conn-same');
  });

  it('sorts CONNECTED equal to ACTIVE above PENDING and EXPIRED', () => {
    const conns = [
      { id: 'exp', toolkit: 'Linear', status: 'EXPIRED', accountEmail: 'a@b.com' },
      { id: 'pending', toolkit: 'Linear', status: 'PENDING', accountEmail: 'a@b.com' },
      { id: 'connected', toolkit: 'Linear', status: 'CONNECTED', accountEmail: 'a@b.com' },
    ];
    const result = deduplicateConnections(conns, 'Account');
    expect(result).toHaveLength(1);
    // CONNECTED ranks same as ACTIVE — must win over EXPIRED and PENDING
    expect(result[0].conn.id).toBe('connected');
  });
});

// ---------------------------------------------------------------------------
// Component tests: ComposioPicker inside the dialog
// ---------------------------------------------------------------------------

describe('AddMemorySourceDialog — Composio picker', () => {
  beforeEach(() => {
    mockListConnections.mockReset();
  });

  it('shows loading state while fetching connections', async () => {
    // Never resolves during this test
    mockListConnections.mockReturnValue(new Promise(() => {}));
    renderDialog();
    fireEvent.click(screen.getByText('Integration'));
    await waitFor(() => expect(screen.queryByText('Loading connections…')).toBeTruthy());
  });

  it('shows no-connections message when list is empty', async () => {
    mockListConnections.mockResolvedValue({ connections: [] });
    await openComposioStep();
    await waitFor(() =>
      expect(
        screen.queryByText('No active Composio connections found. Connect an integration first.')
      ).toBeTruthy()
    );
  });

  it('renders readable labels — toolkit · identity — not raw IDs', async () => {
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'raw-id-xyz', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'user@gmail.com' },
      ],
    });
    await openComposioStep();
    await waitFor(() => expect(screen.queryByText('Gmail · user@gmail.com')).toBeTruthy());
    expect(screen.queryByText('raw-id-xyz')).toBeNull();
  });

  it('deduplicates same toolkit+identity connections in the dropdown', async () => {
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'conn-1', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'x@example.com' },
        { id: 'conn-2', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'x@example.com' },
      ],
    });
    await openComposioStep();
    await waitFor(() => {
      const options = screen.getAllByRole('option');
      const gmailOptions = options.filter(o => o.textContent?.includes('Gmail · x@example.com'));
      expect(gmailOptions).toHaveLength(1);
    });
  });

  it('shows numbered Account labels for connections without identity fields', async () => {
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'conn-a', toolkit: 'Notion', status: 'ACTIVE' },
        { id: 'conn-b', toolkit: 'Notion', status: 'ACTIVE' },
      ],
    });
    await openComposioStep();
    await waitFor(() => {
      expect(screen.queryByText('Notion · Account 1')).toBeTruthy();
      expect(screen.queryByText('Notion · Account 2')).toBeTruthy();
    });
    // Raw IDs must not appear
    expect(screen.queryByText('conn-a')).toBeNull();
    expect(screen.queryByText('conn-b')).toBeNull();
  });

  it('auto-fills the source label when a connection is selected', async () => {
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'conn-1', toolkit: 'Slack', status: 'ACTIVE', workspace: 'my-workspace' },
      ],
    });
    await openComposioStep();
    await waitFor(() => expect(screen.queryByText('Slack · my-workspace')).toBeTruthy());

    const select = screen.getByRole('combobox');
    fireEvent.change(select, { target: { value: 'conn-1' } });

    // The label field should be auto-filled
    await waitFor(() => {
      const labelInput = screen.getByPlaceholderText('My research notes');
      expect((labelInput as HTMLInputElement).value).toBe('Slack · my-workspace');
    });
  });
});
