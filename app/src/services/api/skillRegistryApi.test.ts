import { beforeEach, describe, expect, it, vi } from 'vitest';

import { skillRegistryApi } from './skillRegistryApi';

const mockCallCoreRpc = vi.fn();
vi.mock('../coreRpcClient', () => ({ callCoreRpc: (...a: unknown[]) => mockCallCoreRpc(...a) }));

describe('skillRegistryApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('normalizes install new_skills to newSkills', async () => {
    mockCallCoreRpc.mockResolvedValue({
      url: 'https://example.com/SKILL.md',
      stdout: 'ok',
      stderr: '',
      new_skills: ['demo'],
    });

    const result = await skillRegistryApi.install('demo');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_install',
      params: { entry_id: 'demo' },
    });
    expect(result.newSkills).toEqual(['demo']);
  });

  it('calls skill_registry_uninstall and normalizes removed_path', async () => {
    mockCallCoreRpc.mockResolvedValue({
      name: 'demo',
      removed_path: '/Users/test/.openhuman/skills/demo',
      scope: 'user',
    });

    const result = await skillRegistryApi.uninstall('demo');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_uninstall',
      params: { name: 'demo' },
    });
    expect(result.removedPath).toBe('/Users/test/.openhuman/skills/demo');
  });

  it('fetches skill_registry schemas for smoke script generation', async () => {
    mockCallCoreRpc.mockResolvedValue({
      schemas: [{ namespace: 'skill_registry', function: 'install', inputs: [], outputs: [] }],
    });

    const result = await skillRegistryApi.schemas();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.skill_registry_schemas' });
    expect(result[0].function).toBe('install');
  });

  it('search calls skill_registry_search with query only when source/category absent', async () => {
    mockCallCoreRpc.mockResolvedValue({ entries: [{ id: 'demo', name: 'Demo' }] });

    const result = await skillRegistryApi.search('demo');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_search',
      params: { query: 'demo' },
      timeoutMs: 120_000,
    });
    expect(result[0].id).toBe('demo');
  });

  it('search forwards source and category when both are provided', async () => {
    mockCallCoreRpc.mockResolvedValue({ entries: [] });

    await skillRegistryApi.search('q', 'ClawHub', 'devops');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_search',
      params: { query: 'q', source: 'ClawHub', category: 'devops' },
      timeoutMs: 120_000,
    });
  });

  it('search unwraps data-envelope shape', async () => {
    mockCallCoreRpc.mockResolvedValue({ data: { entries: [{ id: 'env-skill' }] } });

    const result = await skillRegistryApi.search('env');

    expect(result[0].id).toBe('env-skill');
  });

  it('sources calls skill_registry_sources and returns array', async () => {
    mockCallCoreRpc.mockResolvedValue({ sources: ['built-in', 'ClawHub'] });

    const result = await skillRegistryApi.sources();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_sources',
      timeoutMs: 120_000,
    });
    expect(result).toEqual(['built-in', 'ClawHub']);
  });

  it('sources unwraps data-envelope shape', async () => {
    mockCallCoreRpc.mockResolvedValue({ data: { sources: ['optional'] } });

    const result = await skillRegistryApi.sources();

    expect(result).toEqual(['optional']);
  });

  it('categories calls skill_registry_categories and returns array', async () => {
    mockCallCoreRpc.mockResolvedValue({ categories: ['productivity', 'devops'] });

    const result = await skillRegistryApi.categories();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_categories',
      timeoutMs: 120_000,
    });
    expect(result).toEqual(['productivity', 'devops']);
  });

  it('categories unwraps data-envelope shape', async () => {
    mockCallCoreRpc.mockResolvedValue({ data: { categories: ['automation'] } });

    const result = await skillRegistryApi.categories();

    expect(result).toEqual(['automation']);
  });

  it('install falls back to empty newSkills when new_skills is missing', async () => {
    mockCallCoreRpc.mockResolvedValue({
      url: 'https://example.com/SKILL.md',
      stdout: 'ok',
      stderr: '',
      // new_skills deliberately omitted
    });

    const result = await skillRegistryApi.install('demo');

    expect(result.newSkills).toEqual([]);
  });

  it('browse with forceRefresh=true forwards force_refresh=true', async () => {
    mockCallCoreRpc.mockResolvedValue({ entries: [] });

    await skillRegistryApi.browse(true);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_browse',
      params: { force_refresh: true },
      timeoutMs: 120_000,
    });
  });

  it('browse default arg passes force_refresh=false', async () => {
    mockCallCoreRpc.mockResolvedValue({ entries: [] });

    await skillRegistryApi.browse();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_browse',
      params: { force_refresh: false },
      timeoutMs: 120_000,
    });
  });
});
