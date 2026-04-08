/**
 * Skills page — 3rd Party Notion: debug modal tool execution matches
 * `openhuman-skills/src/core/notion/live-test.ts` tool exercises (sections 5b + 7).
 * UI path: Skills → Debug → Tools tab → expand tool → Execute → `openhuman.skills_call_tool`.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

const notionRegistryEntry = {
  id: 'notion',
  name: 'Notion',
  version: '1.2.0',
  description: 'Notion workspace integration.',
  runtime: 'quickjs',
  entry: 'index.js',
  auto_start: false,
  platforms: ['macos', 'linux', 'windows'],
  setup: { required: true, label: 'Connect Notion' },
};

const notionSnapshot = {
  skill_id: 'notion',
  name: 'Notion',
  status: 'running',
  tools: [
    { name: 'sync-status', description: 'Sync status', inputSchema: { type: 'object' } },
    { name: 'list-users', description: 'List users', inputSchema: { type: 'object' } },
    { name: 'search', description: 'Search', inputSchema: { type: 'object' } },
    { name: 'list-pages', description: 'List pages', inputSchema: { type: 'object' } },
    { name: 'list-databases', description: 'List databases', inputSchema: { type: 'object' } },
  ],
  error: null as string | null,
  state: { connection_status: 'connected', auth_status: 'authenticated', is_initialized: true },
  setup_complete: true,
  connection_status: 'connected',
};

const mocks = vi.hoisted(() => ({
  triggerSync: vi.fn().mockResolvedValue(undefined),
  startSkill: vi.fn().mockResolvedValue(undefined),
  callCoreRpc: vi
    .fn()
    .mockResolvedValue({ content: [{ type: 'text', text: '{"ok":true}' }], is_error: false }),
}));

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({ definitions: [], loading: false, error: null }),
}));

vi.mock('../../lib/skills/manager', () => ({
  skillManager: { triggerSync: mocks.triggerSync, startSkill: mocks.startSkill },
}));

vi.mock('../../lib/skills/skillsApi', () => ({
  installSkill: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: mocks.callCoreRpc }));

vi.mock('../../lib/skills/hooks', () => ({
  useAvailableSkills: () => ({ skills: [notionRegistryEntry], loading: false, refresh: vi.fn() }),
  useSkillConnectionStatus: (skillId: string) => (skillId === 'notion' ? 'connected' : 'offline'),
  useSkillState: () => ({
    connection_status: 'connected',
    auth_status: 'authenticated',
    syncInProgress: false,
    syncProgress: 0,
    syncProgressMessage: '',
  }),
  useSkillDataDirectoryStats: () => undefined,
  useSkillSnapshot: (skillId: string | undefined) => (skillId === 'notion' ? notionSnapshot : null),
}));

function expectToolCallArgs(toolName: string, args: Record<string, unknown>) {
  expect(mocks.callCoreRpc).toHaveBeenCalledWith({
    method: 'openhuman.skills_call_tool',
    params: { skill_id: 'notion', tool_name: toolName, arguments: args },
  });
}

describe('Skills page — Notion debug tools (live-test parity)', () => {
  beforeEach(() => {
    mocks.triggerSync.mockClear();
    mocks.startSkill.mockClear();
    mocks.callCoreRpc.mockClear();
  });

  it('opens debug modal and executes tools via openhuman.skills_call_tool', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    expect(screen.getByRole('heading', { name: '3rd Party Skills' })).toBeInTheDocument();
    expect(screen.getByText('Notion')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('skill-debug-button-notion'));

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: /Debug: Notion/i })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId('skill-debug-tab-tools'));

    // --- Section 5b / 7: same tools as live-test.ts (defaults + search query) ---
    const steps: Array<{ tool: string; argsJson?: string }> = [
      { tool: 'sync-status' },
      { tool: 'list-users' },
      { tool: 'search', argsJson: '{"query":"test"}' },
      { tool: 'list-pages', argsJson: '{"page_size":10}' },
      { tool: 'list-databases' },
    ];

    for (const step of steps) {
      mocks.callCoreRpc.mockClear();
      fireEvent.click(screen.getByTestId(`skill-debug-tool-header-${step.tool}`));
      if (step.argsJson) {
        fireEvent.change(screen.getByTestId(`skill-debug-tool-args-${step.tool}`), {
          target: { value: step.argsJson },
        });
      }
      fireEvent.click(screen.getByTestId(`skill-debug-execute-${step.tool}`));
      await waitFor(() => {
        expect(mocks.callCoreRpc).toHaveBeenCalled();
      });
      const parsed =
        step.argsJson !== undefined ? JSON.parse(step.argsJson) : ({} as Record<string, unknown>);
      expectToolCallArgs(step.tool, parsed);
    }

    // --- live-test: repeat list-* with tryCache:false (same tool rows, new args) ---
    mocks.callCoreRpc.mockClear();
    fireEvent.click(screen.getByTestId('skill-debug-tool-header-list-pages'));
    fireEvent.change(screen.getByTestId('skill-debug-tool-args-list-pages'), {
      target: { value: '{"page_size":10,"tryCache":false}' },
    });
    fireEvent.click(screen.getByTestId('skill-debug-execute-list-pages'));
    await waitFor(() => expect(mocks.callCoreRpc).toHaveBeenCalled());
    expectToolCallArgs('list-pages', { page_size: 10, tryCache: false });

    mocks.callCoreRpc.mockClear();
    fireEvent.click(screen.getByTestId('skill-debug-tool-header-list-databases'));
    fireEvent.change(screen.getByTestId('skill-debug-tool-args-list-databases'), {
      target: { value: '{"tryCache":false}' },
    });
    fireEvent.click(screen.getByTestId('skill-debug-execute-list-databases'));
    await waitFor(() => expect(mocks.callCoreRpc).toHaveBeenCalled());
    expectToolCallArgs('list-databases', { tryCache: false });

    mocks.callCoreRpc.mockClear();
    fireEvent.click(screen.getByTestId('skill-debug-tool-header-list-users'));
    fireEvent.change(screen.getByTestId('skill-debug-tool-args-list-users'), {
      target: { value: '{"tryCache":false}' },
    });
    fireEvent.click(screen.getByTestId('skill-debug-execute-list-users'));
    await waitFor(() => expect(mocks.callCoreRpc).toHaveBeenCalled());
    expectToolCallArgs('list-users', { tryCache: false });
  });

  it('Sync control calls skillManager.triggerSync(notion)', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    fireEvent.click(screen.getByTestId('skill-sync-button-notion'));

    await waitFor(() => {
      expect(mocks.triggerSync).toHaveBeenCalledTimes(1);
    });
    expect(mocks.triggerSync).toHaveBeenCalledWith('notion');
  });
});
