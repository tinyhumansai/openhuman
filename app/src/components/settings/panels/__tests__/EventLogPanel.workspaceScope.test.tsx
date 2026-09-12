/**
 * EventLogPanel — the workspace dimension (issue #5966).
 *
 * One core process serves more than one workspace over its life: a switch
 * leaves the previous workspace's subsystems running, and this single stream
 * carries both. Before the envelope named a workspace, a row left over from a
 * workspace the reader had switched away from was indistinguishable from one
 * belonging to the workspace they were in — the log said "MCP server parked"
 * and gave no way to tell whose.
 *
 * Everything here fails silently if it regresses. A dropped scope filter shows
 * another workspace's rows as current; an over-eager one hides the rows the
 * reader came for; and losing track of the active workspace after a switch
 * makes the default view keep presenting stale rows as live. Nothing throws in
 * any of those cases, and the panel keeps looking healthy.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import EventLogPanel from '../EventLogPanel';

vi.mock('../../../../services/coreRpcClient', () => ({
  getCoreHttpBaseUrl: vi.fn().mockResolvedValue('http://localhost:9999'),
  getCoreRpcToken: vi.fn().mockResolvedValue('test-token'),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const WS_A = 'ws_1111111111111111';
const WS_B = 'ws_2222222222222222';

function mockFetchRaw(body: string) {
  const encoder = new TextEncoder();
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(encoder.encode(body));
      controller.close();
    },
  });
  global.fetch = vi.fn().mockResolvedValue({ ok: true, body: stream });
}

/**
 * One row as the core emits it: what workspace it belongs to, and what the
 * core considered active at the moment it was emitted.
 */
const row = (opts: {
  event: string;
  workspace?: string | null;
  active?: string | null;
  domain?: string;
}) =>
  `data:${JSON.stringify({
    domain: opts.domain ?? 'mcp_client',
    event: opts.event,
    agent: '',
    detail: null,
    workspace: opts.workspace ?? null,
    active_workspace: opts.active ?? null,
    timestamp: '12:00:00',
  })}\n\n`;

const configFrame = (payload: Record<string, unknown>) =>
  `event:config\ndata:${JSON.stringify(payload)}\n\n`;

/** Switch the scope select to "all workspaces". */
function selectAllWorkspaces() {
  fireEvent.change(
    screen.getByLabelText('settings.developerMenu.eventLog.workspaceScope') as HTMLSelectElement,
    { target: { value: 'all' } }
  );
}

describe('EventLogPanel workspace scope', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('defaults to the active workspace and hides another workspace’s rows', async () => {
    mockFetchRaw(
      configFrame({ max_entries: 200, new_entries: 'top', active_workspace: WS_A }) +
        row({ event: 'MineParked', workspace: WS_A, active: WS_A }) +
        row({ event: 'TheirsParked', workspace: WS_B, active: WS_A })
    );
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('MineParked')).toBeTruthy());
    expect(screen.queryByText('TheirsParked')).toBeNull();
  });

  it('shows both workspaces when the scope is switched to all', async () => {
    mockFetchRaw(
      configFrame({ max_entries: 200, new_entries: 'top', active_workspace: WS_A }) +
        row({ event: 'MineParked', workspace: WS_A, active: WS_A }) +
        row({ event: 'TheirsParked', workspace: WS_B, active: WS_A })
    );
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('MineParked')).toBeTruthy());
    selectAllWorkspaces();

    await waitFor(() => expect(screen.getByText('TheirsParked')).toBeTruthy());
    expect(screen.getByText('MineParked')).toBeTruthy();
  });

  /**
   * The regression the issue is actually about. The stream stays open across a
   * workspace switch, so rows that were current a moment ago must stop being
   * current — and the ones that were being hidden must start showing. A panel
   * that pinned the active workspace at connect time passes every other test
   * here and fails this one.
   */
  it('re-scopes across a workspace switch without reconnecting', async () => {
    mockFetchRaw(
      configFrame({ max_entries: 200, new_entries: 'top', active_workspace: WS_A }) +
        row({ event: 'BeforeSwitch', workspace: WS_A, active: WS_A }) +
        row({ event: 'AfterSwitch', workspace: WS_B, active: WS_B })
    );
    renderWithProviders(<EventLogPanel />);

    // Once the switch is observed, the workspace the user left drops out and
    // the one they moved to appears.
    await waitFor(() => expect(screen.getByText('AfterSwitch')).toBeTruthy());
    expect(screen.queryByText('BeforeSwitch')).toBeNull();

    // Nothing was discarded — 'all' still has both.
    selectAllWorkspaces();
    await waitFor(() => expect(screen.getByText('BeforeSwitch')).toBeTruthy());
    expect(screen.getByText('AfterSwitch')).toBeTruthy();
  });

  /**
   * Most events are process-wide. Scoping must narrow the log to one
   * workspace, not to the handful of families that happen to name one.
   */
  it('keeps rows that are not bound to any workspace', async () => {
    mockFetchRaw(
      configFrame({ max_entries: 200, new_entries: 'top', active_workspace: WS_A }) +
        row({ event: 'ProcessWide', domain: 'tool', workspace: null, active: WS_A }) +
        row({ event: 'TheirsParked', workspace: WS_B, active: WS_A })
    );
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('ProcessWide')).toBeTruthy());
    expect(screen.queryByText('TheirsParked')).toBeNull();
  });

  /**
   * With no active workspace to compare against — the core could not resolve
   * it — hiding rows would empty the panel and give the reader no way to tell
   * that from a quiet process. Show everything instead.
   */
  it('shows every row while the active workspace is unknown', async () => {
    mockFetchRaw(
      configFrame({ max_entries: 200, new_entries: 'top' }) +
        row({ event: 'FromA', workspace: WS_A, active: null }) +
        row({ event: 'FromB', workspace: WS_B, active: null })
    );
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => expect(screen.getByText('FromA')).toBeTruthy());
    expect(screen.getByText('FromB')).toBeTruthy();
  });

  /**
   * The download is the surface the privacy half of #5966 is about: it leaves
   * the app. It must carry the handle and never the path, and it must export
   * what the reader is looking at rather than the unscoped buffer.
   */
  it('exports only the scoped rows, carrying handles and no filesystem path', async () => {
    mockFetchRaw(
      configFrame({ max_entries: 200, new_entries: 'top', active_workspace: WS_A }) +
        row({ event: 'MineParked', workspace: WS_A, active: WS_A }) +
        row({ event: 'TheirsParked', workspace: WS_B, active: WS_A })
    );

    const blobs: string[] = [];
    const originalBlob = global.Blob;
    class CapturingBlob extends originalBlob {
      constructor(parts: BlobPart[], options?: BlobPropertyBag) {
        blobs.push(parts.map(String).join(''));
        super(parts, options);
      }
    }
    vi.stubGlobal('Blob', CapturingBlob);
    const createObjectURL = vi.fn().mockReturnValue('blob:mock');
    vi.stubGlobal('URL', { ...URL, createObjectURL, revokeObjectURL: vi.fn() });

    try {
      renderWithProviders(<EventLogPanel />);
      await waitFor(() => expect(screen.getByText('MineParked')).toBeTruthy());

      fireEvent.click(screen.getByText('settings.developerMenu.eventLog.download'));

      expect(blobs).toHaveLength(1);
      const exported = blobs[0];
      expect(exported).toContain('MineParked');
      expect(exported).not.toContain('TheirsParked');
      expect(exported).toContain(WS_A);
      // The whole point of the handle: no home directory reaches the file.
      expect(exported).not.toContain('/Users/');
      expect(exported).not.toContain('/home/');
      expect(exported).not.toContain('.openhuman');
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
