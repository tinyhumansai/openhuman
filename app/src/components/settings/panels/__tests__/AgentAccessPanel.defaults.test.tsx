import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type AgentSettings,
  type AutonomySettings,
  isTauri,
  openhumanGetAgentSettings,
  openhumanGetAutonomySettings,
  openhumanUpdateAgentSettings,
  openhumanUpdateAutonomySettings,
} from '../../../../utils/tauriCommands';
import AgentAccessPanel from '../AgentAccessPanel';

/**
 * The fail-safe half of `AgentAccessPanel`.
 *
 * On load the panel reads three security fields through nullish coalescing:
 *
 *   auto_approve_all           ?? false
 *   trusted_roots              ?? []
 *   auto_approve               ?? []
 *
 * Each default is chosen to fail CLOSED — an older core, or one that drops a
 * field, must land on "nothing auto-approved" rather than the permissive
 * value. The existing suite always supplies every field, so none of those
 * arms is exercised otherwise.
 *
 * If `auto_approve_all ?? false` were ever written `?? true`, a core that
 * omitted the field would silently approve every tool call and the toggle
 * would show ON as though the user had chosen it. That is the failure these
 * tests exist to catch.
 *
 * Also covers `addRoot`'s guards (blank, duplicate) and its Enter-key path,
 * which the existing suite reaches only through the Add button.
 */

/** Autonomy settings with the optional security fields deliberately absent. */
function autonomyMissingOptionals(overrides: Partial<AutonomySettings> = {}): AutonomySettings {
  return {
    level: 'supervised',
    workspace_only: false,
    allowed_commands: [],
    forbidden_paths: [],
    allow_tool_install: false,
    max_actions_per_hour: 0,
    ...overrides,
  } as AutonomySettings;
}

const autonomy = (overrides: Partial<AutonomySettings> = {}): AutonomySettings => ({
  level: 'supervised',
  workspace_only: false,
  allowed_commands: [],
  forbidden_paths: [],
  trusted_roots: [],
  allow_tool_install: false,
  max_actions_per_hour: 0,
  auto_approve: [],
  auto_approve_all: false,
  ...overrides,
});

const agentSettings = (overrides: Partial<AgentSettings> = {}): AgentSettings => ({
  agent_timeout_secs: 120,
  effective_timeout_secs: 120,
  env_override: false,
  min_timeout_secs: 1,
  max_timeout_secs: 3600,
  tool_dispatcher: 'auto',
  tool_dispatcher_env_override: false,
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
    openhumanGetAgentSettings: vi.fn(),
    openhumanUpdateAgentSettings: vi.fn(),
    // The "Files folder" row (#5505) reads the agent paths on mount.
    openhumanGetAgentPaths: vi.fn(async () => ({
      result: {
        action_dir: '/home/u/OpenHuman/projects',
        workspace_dir: '/home/u/.openhuman/users/u/workspace',
        projects_dir: '/home/u/OpenHuman/projects',
        action_dir_source: 'default',
        files_dir: '/home/u/OpenHuman/projects/Files',
        default_files_dir: '/home/u/OpenHuman/projects/Files',
        files_dir_source: 'default',
      },
      logs: [],
    })),
    openhumanUpdateAgentPaths: vi.fn(),
  };
});

const mockGet = vi.mocked(openhumanGetAutonomySettings);
const mockUpdate = vi.mocked(openhumanUpdateAutonomySettings);
const mockGetAgent = vi.mocked(openhumanGetAgentSettings);
const mockUpdateAgent = vi.mocked(openhumanUpdateAgentSettings);

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(isTauri).mockReturnValue(true);
  mockGet.mockResolvedValue({ result: autonomy(), logs: [] });
  mockUpdate.mockResolvedValue({ result: {} as never, logs: [] });
  mockGetAgent.mockResolvedValue({ result: agentSettings(), logs: [] });
  mockUpdateAgent.mockResolvedValue({ result: {} as never, logs: [] });
});

describe('AgentAccessPanel — fail-closed defaults for omitted security fields', () => {
  it('leaves auto-approve-all OFF when the core omits the field', async () => {
    mockGet.mockResolvedValue({ result: autonomyMissingOptionals(), logs: [] });
    renderWithProviders(<AgentAccessPanel />);

    await waitFor(() => expect(mockGet).toHaveBeenCalled());
    const autoAll = await screen.findByRole('switch', { name: /auto-approve all|approve all/i });
    // `?? false`: absent must never mean "approve everything".
    expect(autoAll).toHaveAttribute('aria-checked', 'false');
  });

  it('can still add a granted folder when trusted_roots was omitted', async () => {
    // `?? []` has to produce a real array, not just avoid a render crash:
    // `addRoot` spreads it (`[...trustedRoots, next]`), and spreading undefined
    // throws. Adding a root is what actually exercises the default.
    mockGet.mockResolvedValue({ result: autonomyMissingOptionals(), logs: [] });
    renderWithProviders(<AgentAccessPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());

    const pathField = await screen.findByLabelText(/path/i);
    fireEvent.change(pathField, { target: { value: '/srv/first' } });
    fireEvent.keyDown(pathField, { key: 'Enter' });

    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({
          trusted_roots: [expect.objectContaining({ path: '/srv/first' })],
        })
      )
    );
  });

  it('renders with no always-allowed tools when auto_approve is omitted', async () => {
    mockGet.mockResolvedValue({ result: autonomyMissingOptionals(), logs: [] });
    renderWithProviders(<AgentAccessPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());
    // Reaching a rendered panel at all is the assertion: a missing array would
    // throw during render before anything appeared.
    expect(
      await screen.findByRole('switch', { name: /auto-approve all|approve all/i })
    ).toBeInTheDocument();
  });
});

describe('AgentAccessPanel — addRoot guards', () => {
  const pathInput = () => screen.getByLabelText(/path/i);

  async function ready() {
    renderWithProviders(<AgentAccessPanel />);
    await waitFor(() => expect(mockGet).toHaveBeenCalled());
    await waitFor(() => expect(pathInput()).toBeInTheDocument());
  }

  it('adds a root when Enter is pressed in the path field', async () => {
    await ready();
    fireEvent.change(pathInput(), { target: { value: '/srv/data' } });
    fireEvent.keyDown(pathInput(), { key: 'Enter' });

    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({
          trusted_roots: expect.arrayContaining([expect.objectContaining({ path: '/srv/data' })]),
        })
      )
    );
  });

  it('ignores Enter on a blank path', async () => {
    await ready();
    fireEvent.change(pathInput(), { target: { value: '   ' } });
    fireEvent.keyDown(pathInput(), { key: 'Enter' });

    await Promise.resolve();
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('does not add a duplicate root, and clears the field', async () => {
    mockGet.mockResolvedValue({
      result: autonomy({ trusted_roots: [{ path: '/srv/data', access: 'read' }] as never }),
      logs: [],
    });
    await ready();

    fireEvent.change(pathInput(), { target: { value: '/srv/data' } });
    fireEvent.keyDown(pathInput(), { key: 'Enter' });

    await waitFor(() => expect(pathInput()).toHaveValue(''));
    // A duplicate must not be persisted — the list is unchanged, so no RPC.
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('carries the chosen access level onto the new root', async () => {
    await ready();
    const accessSelect = screen.getByLabelText(/access level/i);
    fireEvent.change(accessSelect, { target: { value: 'readwrite' } });
    fireEvent.change(pathInput(), { target: { value: '/srv/rw' } });
    fireEvent.keyDown(pathInput(), { key: 'Enter' });

    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith(
        expect.objectContaining({
          trusted_roots: expect.arrayContaining([
            expect.objectContaining({ path: '/srv/rw', access: 'readwrite' }),
          ]),
        })
      )
    );
  });

  it('does not submit the form on a non-Enter key', async () => {
    await ready();
    fireEvent.change(pathInput(), { target: { value: '/srv/data' } });
    fireEvent.keyDown(pathInput(), { key: 'a' });

    await Promise.resolve();
    expect(mockUpdate).not.toHaveBeenCalled();
  });
});
