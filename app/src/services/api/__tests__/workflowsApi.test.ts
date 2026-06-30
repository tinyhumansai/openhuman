import { beforeEach, describe, expect, it, vi } from 'vitest';

import { workflowsApi } from '../workflowsApi';

vi.mock('../../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('workflowsApi.createWorkflow', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  it('forwards inputs to workflows_create and rekeys allowedTools', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      workflow: {
        id: 'my-skill',
        name: 'my-skill',
        description: 'does stuff',
        version: '',
        author: null,
        tags: ['alpha'],
        tools: ['mcp/fs'],
        prompts: [],
        location: '/home/u/.openhuman/skills/my-skill/SKILL.md',
        resources: [],
        scope: 'user',
        legacy: false,
        warnings: [],
      },
    });

    const result = await workflowsApi.createWorkflow({
      name: 'My Skill',
      description: 'does stuff',
      scope: 'user',
      tags: ['alpha'],
      allowedTools: ['mcp/fs'],
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflows_create',
      params: {
        name: 'My Skill',
        description: 'does stuff',
        scope: 'user',
        tags: ['alpha'],
        'allowed-tools': ['mcp/fs'],
      },
    });
    expect(result.id).toBe('my-skill');
    expect(result.scope).toBe('user');
  });

  it('omits optional fields when not provided', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      workflow: {
        id: 'minimal',
        name: 'minimal',
        description: 'd',
        version: '',
        author: null,
        tags: [],
        tools: [],
        prompts: [],
        location: null,
        resources: [],
        scope: 'user',
        legacy: false,
        warnings: [],
      },
    });

    await workflowsApi.createWorkflow({ name: 'minimal', description: 'd' });

    const call = vi.mocked(callCoreRpc).mock.calls[0][0];
    expect(call.params).toEqual({ name: 'minimal', description: 'd' });
  });

  it('unwraps an envelope response', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      data: {
        workflow: {
          id: 'env',
          name: 'env',
          description: 'e',
          version: '',
          author: null,
          tags: [],
          tools: [],
          prompts: [],
          location: null,
          resources: [],
          scope: 'project',
          legacy: false,
          warnings: [],
        },
      },
    });
    const result = await workflowsApi.createWorkflow({ name: 'env', description: 'e' });
    expect(result.id).toBe('env');
    expect(result.scope).toBe('project');
  });
});

describe('workflowsApi.installWorkflowFromUrl', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  it('forwards url and rekeys timeoutSecs to timeout_secs', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      url: 'https://example.com/my-skill.tgz',
      stdout: 'added my-skill',
      stderr: '',
      new_workflows: ['my-skill'],
    });

    const result = await workflowsApi.installWorkflowFromUrl({
      url: 'https://example.com/my-skill.tgz',
      timeoutSecs: 120,
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflows_install_from_url',
      params: { url: 'https://example.com/my-skill.tgz', timeout_secs: 120 },
    });
    expect(result.newWorkflows).toEqual(['my-skill']);
    expect(result.stdout).toBe('added my-skill');
  });

  it('omits timeout_secs when not provided and normalizes missing new_workflows', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      url: 'https://example.com/x',
      stdout: '',
      stderr: '',
      new_workflows: undefined,
    });

    const result = await workflowsApi.installWorkflowFromUrl({ url: 'https://example.com/x' });

    const call = vi.mocked(callCoreRpc).mock.calls[0][0];
    expect(call.params).toEqual({ url: 'https://example.com/x' });
    expect(result.newWorkflows).toEqual([]);
  });

  it('unwraps an envelope response', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      data: {
        url: 'https://example.com/y',
        stdout: 'ok',
        stderr: 'warn',
        new_workflows: ['y-skill'],
      },
    });
    const result = await workflowsApi.installWorkflowFromUrl({ url: 'https://example.com/y' });
    expect(result.newWorkflows).toEqual(['y-skill']);
    expect(result.stderr).toBe('warn');
  });
});

describe('workflowsApi.updateWorkflow', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  it('forwards every optional field and rekeys allowedTools', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      workflow: { id: 'wf', name: 'WF', description: 'd', scope: 'user' as const },
    });

    await workflowsApi.updateWorkflow({
      name: 'WF',
      description: 'd',
      whenToUse: 'when X happens',
      scope: 'user',
      license: 'MIT',
      author: 'me',
      tags: ['t'],
      allowedTools: ['mcp/fs'],
      inputs: [{ name: 'n', required: true }],
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflows_update',
      params: {
        name: 'WF',
        description: 'd',
        when_to_use: 'when X happens',
        scope: 'user',
        license: 'MIT',
        author: 'me',
        tags: ['t'],
        'allowed-tools': ['mcp/fs'],
        inputs: [{ name: 'n', required: true }],
      },
    });
  });
});

describe('workflowsApi.listWorkflows', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  it('reads the `workflows` result field', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      workflows: [
        { id: 'a', name: 'A', description: '', scope: 'user' as const },
        { id: 'b', name: 'B', description: '', scope: 'project' as const },
      ],
    });

    const result = await workflowsApi.listWorkflows();

    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.workflows_list' });
    expect(result.map(w => w.id)).toEqual(['a', 'b']);
  });

  it('unwraps an envelope and defaults to [] when the field is absent', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ data: {} });
    const result = await workflowsApi.listWorkflows();
    expect(result).toEqual([]);
  });

  it('normalizes snake_case and legacy discovered workflow fields', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      workflows: [
        {
          id: 'hermes-demo',
          name: 'Hermes Demo',
          description: 'Reads Hermes metadata.',
          version: '',
          author: null,
          tags: [],
          related_skills: ['browser-automation'],
          source_format: 'hermes',
          tools: [],
          prompts: [],
          location: null,
          resources: [],
          scope: 'user',
          legacy: false,
          warnings: [],
        },
        {
          id: 'legacy-demo',
          name: 'Legacy Demo',
          description: 'Old package.',
          version: '',
          author: null,
          tags: [],
          tools: [],
          prompts: [],
          location: null,
          resources: [],
          scope: 'user',
          legacy: true,
          warnings: [],
        },
      ],
    });

    const result = await workflowsApi.listWorkflows();

    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.workflows_list' });
    expect(result[0].relatedSkills).toEqual(['browser-automation']);
    expect(result[0].sourceFormat).toBe('hermes');
    expect(result[0].platforms).toEqual([]);
    expect(result[1].sourceFormat).toBe('legacy');
  });

  it('omits params by default (automations-only view)', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ workflows: [] });
    await workflowsApi.listWorkflows();
    const call = vi.mocked(callCoreRpc).mock.calls[0][0];
    expect(call.method).toBe('openhuman.workflows_list');
    expect(call.params).toBeUndefined();
  });

  it('passes include_skills when includeSkills is set', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ workflows: [] });
    await workflowsApi.listWorkflows({ includeSkills: true });
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflows_list',
      params: { include_skills: true },
    });
  });
});

describe('workflowsApi.readWorkflowResource', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  it('rekeys params to snake_case and normalizes the response', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      workflow_id: 'wf',
      relative_path: 'scripts/run.sh',
      content: '#!/bin/sh\n',
      bytes: 10,
    });

    const result = await workflowsApi.readWorkflowResource({
      workflowId: 'wf',
      relativePath: 'scripts/run.sh',
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.workflows_read_resource',
      params: { workflow_id: 'wf', relative_path: 'scripts/run.sh' },
    });
    expect(result).toEqual({
      workflowId: 'wf',
      relativePath: 'scripts/run.sh',
      content: '#!/bin/sh\n',
      bytes: 10,
    });
  });
});

describe('workflowsApi.uninstallWorkflow', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  it('forwards the name and normalizes removed_path → removedPath', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      name: 'weather-helper',
      removed_path: '/home/u/.openhuman/skills/weather-helper',
      scope: 'user',
    });

    const result = await workflowsApi.uninstallWorkflow('weather-helper');

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_uninstall',
      params: { name: 'weather-helper' },
    });
    expect(result).toEqual({
      name: 'weather-helper',
      removedPath: '/home/u/.openhuman/skills/weather-helper',
      scope: 'user',
    });
  });
});
