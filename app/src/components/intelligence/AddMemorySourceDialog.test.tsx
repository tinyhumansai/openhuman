/**
 * Tests for the AddMemorySourceDialog — focused on the Composio connection
 * picker: deduplication, readable labels, and no raw connection IDs in the
 * rendered dropdown (issue #3356).
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { listConnections } from '../../lib/composio/composioApi';
import { getSupportedToolkits } from '../../services/memorySourcesService';
import { renderWithProviders } from '../../test/test-utils';
import { AddMemorySourceDialog, deduplicateConnections } from './AddMemorySourceDialog';

// ---------------------------------------------------------------------------
// Module mocks
// ---------------------------------------------------------------------------

vi.mock('../../lib/composio/composioApi', () => ({ listConnections: vi.fn() }));

vi.mock('../../services/memorySourcesService', () => ({
  addMemorySource: vi.fn(),
  getSupportedToolkits: vi.fn(),
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
const mockGetSupportedToolkits = getSupportedToolkits as ReturnType<typeof vi.fn>;

/** Every toolkit used across the picker component tests is syncable by default,
 *  so existing assertions keep passing. Tests that exercise the disabled /
 *  "Coming soon" path override this with a narrower set. */
const DEFAULT_SUPPORTED = ['gmail', 'slack', 'notion', 'github', 'linear', 'clickup'];

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

/** Open the custom connection dropdown so its option rows render. */
async function openListbox() {
  const trigger = await screen.findByTestId('composio-connection-picker');
  fireEvent.click(trigger);
  await screen.findByTestId('composio-connection-listbox');
}

// ---------------------------------------------------------------------------
// Unit tests: deduplicateConnections helper
// ---------------------------------------------------------------------------

describe('deduplicateConnections', () => {
  it('returns an empty array for empty input', () => {
    expect(deduplicateConnections([])).toEqual([]);
  });

  it('uses accountEmail as the identity label', () => {
    const conn = {
      id: 'conn-1',
      toolkit: 'Gmail',
      status: 'ACTIVE',
      accountEmail: 'user@example.com',
    };
    const result = deduplicateConnections([conn]);
    expect(result).toHaveLength(1);
    expect(result[0].label).toBe('Gmail · user@example.com');
    expect(result[0].conn.id).toBe('conn-1');
  });

  it('falls back to workspace when accountEmail is absent', () => {
    const conn = { id: 'conn-2', toolkit: 'Slack', status: 'ACTIVE', workspace: 'my-workspace' };
    const result = deduplicateConnections([conn]);
    expect(result[0].label).toBe('Slack · my-workspace');
  });

  it('falls back to username when neither email nor workspace is present', () => {
    const conn = { id: 'conn-3', toolkit: 'GitHub', status: 'ACTIVE', username: 'octocat' };
    const result = deduplicateConnections([conn]);
    expect(result[0].label).toBe('GitHub · octocat');
  });

  it('uses connection ID as label when no identity field is available', () => {
    const conn = { id: 'conn-x', toolkit: 'Notion', status: 'ACTIVE' };
    const result = deduplicateConnections([conn]);
    expect(result[0].label).toBe('Notion · conn-x');
  });

  it('shows each connection ID for multiple no-identity connections', () => {
    const conns = [
      { id: 'conn-a', toolkit: 'Notion', status: 'ACTIVE' },
      { id: 'conn-b', toolkit: 'Notion', status: 'ACTIVE' },
    ];
    const result = deduplicateConnections(conns);
    expect(result).toHaveLength(2);
    expect(result[0].label).toBe('Notion · conn-a');
    expect(result[1].label).toBe('Notion · conn-b');
  });

  it('deduplicates connections with the same toolkit and identity', () => {
    const conns = [
      { id: 'conn-1', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'a@example.com' },
      { id: 'conn-2', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'a@example.com' },
    ];
    const result = deduplicateConnections(conns);
    expect(result).toHaveLength(1);
    expect(result[0].conn.id).toBe('conn-1');
    expect(result[0].label).toBe('Gmail · a@example.com');
  });

  it('keeps connections with the same toolkit but different identities', () => {
    const conns = [
      { id: 'conn-1', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'a@example.com' },
      { id: 'conn-2', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'b@example.com' },
    ];
    const result = deduplicateConnections(conns);
    expect(result).toHaveLength(2);
  });

  it('uses the connection ID in the label when no identity is available', () => {
    const conns = [
      { id: 'raw-uuid-abc123', toolkit: 'Linear', status: 'ACTIVE' },
      { id: 'raw-uuid-def456', toolkit: 'Linear', status: 'ACTIVE' },
    ];
    const result = deduplicateConnections(conns);
    expect(result[0].label).toBe('Linear · raw-uuid-abc123');
    expect(result[1].label).toBe('Linear · raw-uuid-def456');
  });

  it('shows connection IDs for no-identity connections across toolkits', () => {
    const conns = [
      { id: 'n-1', toolkit: 'Notion', status: 'ACTIVE' },
      { id: 's-1', toolkit: 'Slack', status: 'ACTIVE' },
      { id: 'n-2', toolkit: 'Notion', status: 'ACTIVE' },
    ];
    const result = deduplicateConnections(conns);
    expect(result).toHaveLength(3);
    expect(result.find(r => r.conn.id === 'n-1')?.label).toBe('Notion · n-1');
    expect(result.find(r => r.conn.id === 'n-2')?.label).toBe('Notion · n-2');
    expect(result.find(r => r.conn.id === 's-1')?.label).toBe('Slack · s-1');
  });

  it('prefers ACTIVE over EXPIRED when deduplicating same toolkit+identity', () => {
    // Backend returns EXPIRED first — the ACTIVE one should win
    const conns = [
      { id: 'conn-expired', toolkit: 'Gmail', status: 'EXPIRED', accountEmail: 'x@example.com' },
      { id: 'conn-active', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'x@example.com' },
    ];
    const result = deduplicateConnections(conns);
    expect(result).toHaveLength(1);
    expect(result[0].conn.id).toBe('conn-active');
  });

  it('deduplicates identity-less connections with the same conn.id', () => {
    // Same connection returned twice with no identity — must not produce duplicate React keys
    const conns = [
      { id: 'conn-same', toolkit: 'Notion', status: 'ACTIVE' },
      { id: 'conn-same', toolkit: 'Notion', status: 'ACTIVE' },
    ];
    const result = deduplicateConnections(conns);
    expect(result).toHaveLength(1);
    expect(result[0].conn.id).toBe('conn-same');
  });

  it('sorts CONNECTED equal to ACTIVE above PENDING and EXPIRED', () => {
    const conns = [
      { id: 'exp', toolkit: 'Linear', status: 'EXPIRED', accountEmail: 'a@b.com' },
      { id: 'pending', toolkit: 'Linear', status: 'PENDING', accountEmail: 'a@b.com' },
      { id: 'connected', toolkit: 'Linear', status: 'CONNECTED', accountEmail: 'a@b.com' },
    ];
    const result = deduplicateConnections(conns);
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
    mockGetSupportedToolkits.mockReset();
    mockGetSupportedToolkits.mockResolvedValue(DEFAULT_SUPPORTED);
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
    await openListbox();
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
    await openListbox();
    await waitFor(() => {
      const options = screen.getAllByRole('option');
      const gmailOptions = options.filter(o => o.textContent?.includes('Gmail · x@example.com'));
      expect(gmailOptions).toHaveLength(1);
    });
  });

  it('shows connection IDs for connections without identity fields', async () => {
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'conn-a', toolkit: 'Notion', status: 'ACTIVE' },
        { id: 'conn-b', toolkit: 'Notion', status: 'ACTIVE' },
      ],
    });
    await openComposioStep();
    await openListbox();
    await waitFor(() => {
      expect(screen.queryByText('Notion · conn-a')).toBeTruthy();
      expect(screen.queryByText('Notion · conn-b')).toBeTruthy();
    });
  });

  it('auto-fills the source label when a connection is selected', async () => {
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'conn-1', toolkit: 'Slack', status: 'ACTIVE', workspace: 'my-workspace' },
      ],
    });
    await openComposioStep();
    await openListbox();
    const option = await screen.findByTestId('composio-option-conn-1');
    fireEvent.click(option);

    // The label field should be auto-filled, and the dropdown collapses to
    // show the chosen connection on the trigger.
    await waitFor(() => {
      const labelInput = screen.getByPlaceholderText('My research notes');
      expect((labelInput as HTMLInputElement).value).toBe('Slack · my-workspace');
    });
  });

  it('disables unsupported toolkits with a "Coming soon" tag and keeps them unselectable', async () => {
    // Slack is syncable; Google Calendar is not in the supported set.
    mockGetSupportedToolkits.mockResolvedValue(['slack']);
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'conn-slack', toolkit: 'slack', status: 'ACTIVE', workspace: 'acme' },
        { id: 'conn-gcal', toolkit: 'googlecalendar', status: 'ACTIVE', accountEmail: 'a@x.com' },
      ],
    });
    await openComposioStep();
    await openListbox();

    const supported = await screen.findByTestId('composio-option-conn-slack');
    const unsupported = await screen.findByTestId('composio-option-conn-gcal');

    expect(supported.getAttribute('aria-disabled')).toBe('false');
    expect(unsupported.getAttribute('aria-disabled')).toBe('true');
    expect(screen.getByTestId('composio-option-coming-soon-conn-gcal')).toBeInTheDocument();
    // No "Coming soon" chip on the supported row.
    expect(screen.queryByTestId('composio-option-coming-soon-conn-slack')).toBeNull();

    // Clicking the unsupported row must NOT select it (label stays empty).
    fireEvent.click(unsupported);
    const labelInput = screen.getByPlaceholderText('My research notes');
    expect((labelInput as HTMLInputElement).value).toBe('');

    // Clicking the supported row selects it.
    fireEvent.click(supported);
    await waitFor(() => expect((labelInput as HTMLInputElement).value).toBe('slack · acme'));
  });

  it('treats every connection as supported when the supported-toolkit RPC fails', async () => {
    // Fallback path: getSupportedToolkits rejects → null set → nothing disabled.
    mockGetSupportedToolkits.mockRejectedValue(new Error('rpc down'));
    mockListConnections.mockResolvedValue({
      connections: [{ id: 'conn-x', toolkit: 'sentry', status: 'ACTIVE', accountEmail: 'a@x.com' }],
    });
    await openComposioStep();
    await openListbox();
    const option = await screen.findByTestId('composio-option-conn-x');
    expect(option.getAttribute('aria-disabled')).toBe('false');
    expect(screen.queryByTestId('composio-option-coming-soon-conn-x')).toBeNull();
  });

  it('closes the dropdown when Escape is pressed', async () => {
    mockListConnections.mockResolvedValue({
      connections: [{ id: 'conn-1', toolkit: 'Slack', status: 'ACTIVE', workspace: 'acme' }],
    });
    await openComposioStep();
    await openListbox();
    expect(screen.queryByTestId('composio-connection-listbox')).toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('composio-connection-listbox')).toBeNull());
  });

  it('closes the dropdown on an outside click', async () => {
    mockListConnections.mockResolvedValue({
      connections: [{ id: 'conn-1', toolkit: 'Slack', status: 'ACTIVE', workspace: 'acme' }],
    });
    await openComposioStep();
    await openListbox();
    expect(screen.queryByTestId('composio-connection-listbox')).toBeInTheDocument();

    // A mousedown outside the picker container collapses the listbox.
    fireEvent.mouseDown(document.body);
    await waitFor(() => expect(screen.queryByTestId('composio-connection-listbox')).toBeNull());
  });

  it('opens the listbox with ArrowDown on the trigger button', async () => {
    mockListConnections.mockResolvedValue({
      connections: [{ id: 'conn-1', toolkit: 'Slack', status: 'ACTIVE', workspace: 'acme' }],
    });
    await openComposioStep();
    const trigger = await screen.findByTestId('composio-connection-picker');
    fireEvent.keyDown(trigger, { key: 'ArrowDown' });
    await screen.findByTestId('composio-connection-listbox');
  });

  it('navigates options with the arrow keys and selects with Enter', async () => {
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'conn-gmail', toolkit: 'Gmail', status: 'ACTIVE', accountEmail: 'a@x.com' },
        { id: 'conn-slack', toolkit: 'Slack', status: 'ACTIVE', workspace: 'acme' },
      ],
    });
    await openComposioStep();
    await openListbox();
    const listbox = screen.getByTestId('composio-connection-listbox');

    // Opens highlighting the first selectable option; ArrowDown moves to the next.
    expect(listbox).toHaveAttribute('aria-activedescendant', 'composio-opt-conn-gmail');
    fireEvent.keyDown(listbox, { key: 'ArrowDown' });
    expect(listbox).toHaveAttribute('aria-activedescendant', 'composio-opt-conn-slack');

    // Enter selects the highlighted option and closes the dropdown.
    fireEvent.keyDown(listbox, { key: 'Enter' });
    await waitFor(() => {
      const labelInput = screen.getByPlaceholderText('My research notes');
      expect((labelInput as HTMLInputElement).value).toBe('Slack · acme');
    });
    expect(screen.queryByTestId('composio-connection-listbox')).toBeNull();
  });

  it('skips unsupported options during keyboard navigation', async () => {
    mockGetSupportedToolkits.mockResolvedValue(['slack']);
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'conn-slack', toolkit: 'slack', status: 'ACTIVE', workspace: 'acme' },
        { id: 'conn-gcal', toolkit: 'googlecalendar', status: 'ACTIVE', accountEmail: 'a@x.com' },
      ],
    });
    await openComposioStep();
    await openListbox();
    const listbox = screen.getByTestId('composio-connection-listbox');

    // Only the supported option is reachable; wrapping keeps it on slack.
    expect(listbox).toHaveAttribute('aria-activedescendant', 'composio-opt-conn-slack');
    fireEvent.keyDown(listbox, { key: 'ArrowDown' });
    expect(listbox).toHaveAttribute('aria-activedescendant', 'composio-opt-conn-slack');
    fireEvent.keyDown(listbox, { key: 'ArrowUp' });
    expect(listbox).toHaveAttribute('aria-activedescendant', 'composio-opt-conn-slack');
  });
});
