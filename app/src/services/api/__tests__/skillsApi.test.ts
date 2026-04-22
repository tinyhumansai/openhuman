import { beforeEach, describe, expect, it, vi } from 'vitest';

import { skillsApi } from '../skillsApi';

vi.mock('../../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('skillsApi.createSkill', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  it('forwards inputs to skills_create and rekeys allowedTools', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      skill: {
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

    const result = await skillsApi.createSkill({
      name: 'My Skill',
      description: 'does stuff',
      scope: 'user',
      tags: ['alpha'],
      allowedTools: ['mcp/fs'],
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skills_create',
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
      skill: {
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

    await skillsApi.createSkill({ name: 'minimal', description: 'd' });

    const call = vi.mocked(callCoreRpc).mock.calls[0][0];
    expect(call.params).toEqual({ name: 'minimal', description: 'd' });
  });

  it('unwraps an envelope response', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      data: {
        skill: {
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
    const result = await skillsApi.createSkill({ name: 'env', description: 'e' });
    expect(result.id).toBe('env');
    expect(result.scope).toBe('project');
  });
});

describe('skillsApi.installSkillFromUrl', () => {
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
      new_skills: ['my-skill'],
    });

    const result = await skillsApi.installSkillFromUrl({
      url: 'https://example.com/my-skill.tgz',
      timeoutSecs: 120,
    });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skills_install_from_url',
      params: { url: 'https://example.com/my-skill.tgz', timeout_secs: 120 },
    });
    expect(result.newSkills).toEqual(['my-skill']);
    expect(result.stdout).toBe('added my-skill');
  });

  it('omits timeout_secs when not provided and normalizes missing new_skills', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      url: 'https://example.com/x',
      stdout: '',
      stderr: '',
      new_skills: undefined,
    });

    const result = await skillsApi.installSkillFromUrl({ url: 'https://example.com/x' });

    const call = vi.mocked(callCoreRpc).mock.calls[0][0];
    expect(call.params).toEqual({ url: 'https://example.com/x' });
    expect(result.newSkills).toEqual([]);
  });

  it('unwraps an envelope response', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      data: { url: 'https://example.com/y', stdout: 'ok', stderr: 'warn', new_skills: ['y-skill'] },
    });
    const result = await skillsApi.installSkillFromUrl({ url: 'https://example.com/y' });
    expect(result.newSkills).toEqual(['y-skill']);
    expect(result.stderr).toBe('warn');
  });
});
