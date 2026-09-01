/**
 * PermissionsPanel — the sequence guards, the RPC-shape defaults, and the two
 * save-failure branches.
 *
 * `PermissionsPanel.test.tsx` (sibling) covers the three presets, the
 * action_dir edit flow and the env-locked notice. What it does not reach is the
 * panel's concurrency machinery: `persistSeqRef` (`PermissionsPanel.tsx:115`)
 * and `dirSeqRef` (`:166`) exist so a slow in-flight response cannot overwrite
 * the result of a newer one, and neither guard has a test. Branch coverage sat
 * at 69.8% with those and the `??` defaults unexercised.
 *
 * Separate file rather than an edit to the sibling: three of us are in this
 * directory.
 */
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type AgentPaths,
  type AutonomySettings,
  isTauri,
  openhumanGetAgentPaths,
  openhumanGetAutonomySettings,
  openhumanUpdateAgentPaths,
  openhumanUpdateAutonomySettings,
} from '../../../../utils/tauriCommands';
import PermissionsPanel from '../PermissionsPanel';

const autonomy = (overrides: Partial<AutonomySettings> = {}): AutonomySettings => ({
  level: 'supervised',
  workspace_only: false,
  allowed_commands: [],
  forbidden_paths: [],
  trusted_roots: [],
  allow_tool_install: true,
  max_actions_per_hour: 0,
  auto_approve: [],
  ...overrides,
});

const agentPaths = (overrides: Partial<AgentPaths> = {}): AgentPaths => ({
  action_dir: '/home/test/OpenHuman/projects',
  workspace_dir: '/home/test/.openhuman/users/u1/workspace',
  projects_dir: '/home/test/OpenHuman/projects',
  action_dir_source: 'default',
  ...overrides,
});

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/tauriCommands')>(
    '../../../../utils/tauriCommands'
  );
  return {
    ...actual,
    isTauri: vi.fn(() => true),
    openhumanGetAutonomySettings: vi.fn(),
    openhumanUpdateAutonomySettings: vi.fn(),
    openhumanGetAgentPaths: vi.fn(),
    openhumanUpdateAgentPaths: vi.fn(),
  };
});

const mockGet = vi.mocked(openhumanGetAutonomySettings);
const mockUpdate = vi.mocked(openhumanUpdateAutonomySettings);
const mockGetPaths = vi.mocked(openhumanGetAgentPaths);
const mockUpdatePaths = vi.mocked(openhumanUpdateAgentPaths);
const mockIsTauri = vi.mocked(isTauri);

/** A promise plus the handles to settle it later — lets a test hold one RPC
 *  in flight while a second one starts and finishes. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Let a settled promise's `.then`/`.catch` chain and React's resulting render
 *  actually run. A bare `waitFor(() => expect(x).not.toBeInTheDocument())`
 *  passes on its FIRST tick — before the rejection has been handled — so it
 *  would hold even with the guard removed. This makes the negative assertions
 *  below mean something. */
async function flush() {
  await act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0));
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}

const preset = (name: RegExp) => screen.getByText(name).closest('button') as HTMLElement;

describe('PermissionsPanel — load-shape defaults', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    mockGetPaths.mockResolvedValue({ result: agentPaths(), logs: [] });
  });

  // `require_task_plan_approval ?? true` and `trusted_roots ?? []`
  // (`PermissionsPanel.tsx:90-91`) exist because an older core omits both
  // fields. The defaults matter: they are carried straight back into the next
  // save, so getting them wrong silently rewrites the user's settings.
  it('defaults the fields an older core omits, and carries them into a save', async () => {
    const partial = autonomy();
    delete (partial as Partial<AutonomySettings>).trusted_roots;
    delete (partial as { require_task_plan_approval?: boolean }).require_task_plan_approval;
    mockGet.mockResolvedValue({ result: partial as AutonomySettings, logs: [] });
    mockUpdate.mockResolvedValue({ result: {} as never, logs: [] });

    renderWithProviders(<PermissionsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());

    fireEvent.click(preset(/Full control/i));

    await waitFor(() => expect(mockUpdate).toHaveBeenCalled());
    const sent = mockUpdate.mock.calls[0][0];
    expect(sent.require_task_plan_approval).toBe(true);
    expect(sent.trusted_roots).toEqual([]);
  });

  it('preserves a false require_task_plan_approval rather than defaulting it on', async () => {
    mockGet.mockResolvedValue({
      result: autonomy({ require_task_plan_approval: false } as Partial<AutonomySettings>),
      logs: [],
    });
    mockUpdate.mockResolvedValue({ result: {} as never, logs: [] });

    renderWithProviders(<PermissionsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());

    fireEvent.click(preset(/Full control/i));

    await waitFor(() => expect(mockUpdate).toHaveBeenCalled());
    expect(mockUpdate.mock.calls[0][0].require_task_plan_approval).toBe(false);
  });

  it('reports an autonomy load failure but still renders the folder section', async () => {
    mockGet.mockRejectedValue(new Error('autonomy rpc down'));

    renderWithProviders(<PermissionsPanel />);

    expect(await screen.findByText(/autonomy rpc down/)).toBeInTheDocument();
    // The paths RPC is independent — a failed autonomy load must not blank it.
    await waitFor(() =>
      expect(screen.getByText('/home/test/OpenHuman/projects')).toBeInTheDocument()
    );
  });
});

describe('PermissionsPanel — persist sequence guard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    mockGet.mockResolvedValue({ result: autonomy(), logs: [] });
    mockGetPaths.mockResolvedValue({ result: agentPaths(), logs: [] });
  });

  it('surfaces a save failure', async () => {
    mockUpdate.mockRejectedValue(new Error('autonomy save refused'));

    renderWithProviders(<PermissionsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());

    fireEvent.click(preset(/Full control/i));

    expect(await screen.findByText(/autonomy save refused/)).toBeInTheDocument();
  });

  // The guard's whole purpose: an older in-flight save must not report its
  // outcome once a newer one has started. Without `persistSeqRef` the stale
  // rejection below would paint an error over a save that actually succeeded.
  it('ignores a stale save failure that lands after a newer save', async () => {
    const stale = deferred<{ result: never; logs: string[] }>();
    mockUpdate.mockReturnValueOnce(stale.promise);
    mockUpdate.mockResolvedValueOnce({ result: {} as never, logs: [] });

    renderWithProviders(<PermissionsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());

    fireEvent.click(preset(/Full control/i));
    fireEvent.click(preset(/Look, don't touch/i));
    await waitFor(() => expect(mockUpdate).toHaveBeenCalledTimes(2));

    // Now let the FIRST call fail, after the second has already finished.
    stale.reject(new Error('stale failure'));
    await flush();
    expect(screen.queryByText(/stale failure/)).not.toBeInTheDocument();
  });

  it('ignores a stale save success that lands after a newer failure', async () => {
    const stale = deferred<{ result: never; logs: string[] }>();
    mockUpdate.mockReturnValueOnce(stale.promise);
    mockUpdate.mockRejectedValueOnce(new Error('newer save refused'));

    renderWithProviders(<PermissionsPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());

    fireEvent.click(preset(/Full control/i));
    fireEvent.click(preset(/Look, don't touch/i));

    expect(await screen.findByText(/newer save refused/)).toBeInTheDocument();

    stale.resolve({ result: {} as never, logs: [] });
    await flush();
    // The newer failure must still stand — a late success must not clear it...
    expect(screen.getByText(/newer save refused/)).toBeInTheDocument();
    // ...and no "Saved" note appears over a save that failed.
    //
    // Defence in depth, and worth being precise about: this holds for TWO
    // independent reasons — `persistSeqRef` drops the stale success before it
    // reaches state, and `StatusLine` renders error-over-saving-over-saved by
    // design ("so a failure is never masked by a stale success note",
    // `ui/StatusLine.tsx:16`). Removing the seq guard alone leaves this green,
    // which I verified. It fails only if both go, so read it as a guard on the
    // user-visible outcome, not on `persistSeqRef` specifically.
    expect(screen.queryByText(/Saved: applies on your next message/)).not.toBeInTheDocument();
  });
});

describe('PermissionsPanel — action_dir sequence guard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    mockGet.mockResolvedValue({ result: autonomy(), logs: [] });
    mockGetPaths.mockResolvedValue({ result: agentPaths(), logs: [] });
  });

  const startEditing = async () => {
    renderWithProviders(<PermissionsPanel />);
    await waitFor(() => expect(mockGetPaths).toHaveBeenCalled());
    fireEvent.click(await screen.findByText('Edit'));
    return screen.getByDisplayValue('/home/test/OpenHuman/projects');
  };

  it('trims the typed path before sending it', async () => {
    mockUpdatePaths.mockResolvedValue({ result: agentPaths({ action_dir: '/new/dir' }), logs: [] });

    const input = await startEditing();
    fireEvent.change(input, { target: { value: '   /new/dir   ' } });
    fireEvent.click(screen.getByText('Save'));

    await waitFor(() => expect(mockUpdatePaths).toHaveBeenCalledWith({ action_dir: '/new/dir' }));
  });

  it('leaves edit mode open when the save fails', async () => {
    mockUpdatePaths.mockRejectedValue(new Error('path rejected by core'));

    const input = await startEditing();
    fireEvent.change(input, { target: { value: '/bad/dir' } });
    fireEvent.click(screen.getByText('Save'));

    expect(await screen.findByText(/path rejected by core/)).toBeInTheDocument();
    // Still editing — the user's typed value must not be thrown away.
    expect(screen.getByDisplayValue('/bad/dir')).toBeInTheDocument();
  });

  it('discards the edit and restores the original on cancel', async () => {
    const input = await startEditing();
    fireEvent.change(input, { target: { value: '/scratch' } });
    fireEvent.click(screen.getByText('Cancel'));

    expect(screen.queryByDisplayValue('/scratch')).not.toBeInTheDocument();
    expect(screen.getByText('/home/test/OpenHuman/projects')).toBeInTheDocument();
  });

  // `dirSeqRef` (`PermissionsPanel.tsx:166`) guards against an overlapping
  // action_dir save, but the UI makes that unreachable: Save is disabled while
  // one is in flight. Assert the guard that actually holds — a second click
  // cannot start a second RPC — rather than a race the user cannot trigger.
  it('disables Save while one is in flight, so a second click cannot double-send', async () => {
    const inflight = deferred<{ result: AgentPaths; logs: string[] }>();
    mockUpdatePaths.mockReturnValueOnce(inflight.promise);

    const input = await startEditing();
    fireEvent.change(input, { target: { value: '/first' } });
    const save = screen.getByText('Save').closest('button') as HTMLButtonElement;
    fireEvent.click(save);

    await waitFor(() => expect(save).toBeDisabled());
    fireEvent.click(save);
    expect(mockUpdatePaths).toHaveBeenCalledTimes(1);

    inflight.resolve({ result: agentPaths({ action_dir: '/first' }), logs: [] });
    await waitFor(() => expect(screen.getByText('/first')).toBeInTheDocument());
  });
});

describe('PermissionsPanel — off-Tauri', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(false);
  });

  it('does not persist a tier change in the browser', async () => {
    renderWithProviders(<PermissionsPanel />);

    fireEvent.click(preset(/Full control/i));

    await waitFor(() => expect(screen.getByText(/desktop app/i)).toBeInTheDocument());
    expect(mockUpdate).not.toHaveBeenCalled();
  });
});
