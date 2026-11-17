import { beforeEach, describe, expect, it, vi } from 'vitest';

import { isInstallable, parseRegistryError, skillRegistryApi } from './skillRegistryApi';

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
      // Locating a skills.sh skill plus the 60s fetch outlasts the default 30s.
      timeoutMs: 120_000,
    });
    expect(result).toMatchObject({ status: 'installed', newSkills: ['demo'] });
  });

  it('returns scan_blocked and sends the acknowledgement only when asked', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      status: 'scan_blocked',
      target: 'demo',
      fetched_from: 'https://example.com/SKILL.md',
      slug: 'demo',
      digest: 'abc123',
      findings: [{ check: 'hardcoded_credential', verdict: 'block', field: 'body', message: 'm' }],
      message: 'blocked',
    });
    const blocked = await skillRegistryApi.install('demo');
    expect(blocked.status).toBe('scan_blocked');
    expect(blocked.status === 'scan_blocked' && blocked.scan.findings).toHaveLength(1);
    expect(blocked.status === 'scan_blocked' && blocked.scan.digest).toBe('abc123');
    expect(mockCallCoreRpc.mock.calls[0][0].params).toEqual({ entry_id: 'demo' });

    mockCallCoreRpc.mockResolvedValueOnce({
      status: 'installed',
      url: 'u',
      stdout: '',
      stderr: '',
      new_skills: ['demo'],
    });
    const installed = await skillRegistryApi.install('demo', { acknowledgedDigest: 'abc123' });
    expect(installed.status).toBe('installed');
    expect(mockCallCoreRpc.mock.calls[1][0].params).toEqual({
      entry_id: 'demo',
      acknowledged_digest: 'abc123',
    });
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

    expect(result).toMatchObject({ newSkills: [] });
  });

  it('browsePage asks browse for an empty query and normalizes the page', async () => {
    mockCallCoreRpc.mockResolvedValue({
      entries: [{ id: 'a', name: 'A' }],
      total: 30,
      page: 2,
      page_size: 25,
      total_pages: 2,
      freshness: 'cached',
      fetched_at: 1_700_000_000,
      refreshing: true,
      last_error: { kind: 'unavailable', message: 'upstream returned status 503' },
    });

    const page = await skillRegistryApi.browsePage({ page: 2, pageSize: 25 });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_browse',
      params: { page: 2, page_size: 25 },
      timeoutMs: 120_000,
    });
    expect(page).toMatchObject({
      total: 30,
      page: 2,
      pageSize: 25,
      totalPages: 2,
      freshness: 'cached',
      fetchedAt: 1_700_000_000,
      refreshing: true,
      lastError: { kind: 'unavailable' },
    });
  });

  it('browsePage searches with a query and forwards sources, category and refresh', async () => {
    mockCallCoreRpc.mockResolvedValue({
      data: { entries: [], total: 0, page: 1, page_size: 10, total_pages: 0, freshness: 'live' },
    });

    const page = await skillRegistryApi.browsePage({
      query: ' git ',
      sources: ['ClawHub', 'skills.sh'],
      category: 'devops',
      page: 1,
      pageSize: 10,
      forceRefresh: true,
    });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_search',
      params: {
        query: 'git',
        sources: ['ClawHub', 'skills.sh'],
        category: 'devops',
        page: 1,
        page_size: 10,
        force_refresh: true,
      },
      timeoutMs: 120_000,
    });
    expect(page.lastError).toBeNull();
    expect(page.fetchedAt).toBeNull();
  });

  it('every page read goes to the core; nothing is cached in the app', async () => {
    mockCallCoreRpc.mockResolvedValue({
      entries: [],
      total: 0,
      page: 1,
      page_size: 25,
      total_pages: 0,
      freshness: 'live',
    });

    await skillRegistryApi.browsePage({ page: 1, pageSize: 25 });
    await skillRegistryApi.browsePage({ page: 1, pageSize: 25 });

    expect(mockCallCoreRpc).toHaveBeenCalledTimes(2);
  });

  it('detail calls skill_registry_detail', async () => {
    mockCallCoreRpc.mockResolvedValue({ id: 'git-helper', overview: 'Longer text.' });

    const detail = await skillRegistryApi.detail('git-helper');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.skill_registry_detail',
      params: { entry_id: 'git-helper' },
      timeoutMs: 120_000,
    });
    expect(detail.overview).toBe('Longer text.');
  });

  it('parseRegistryError reads the kind, message and retry delay', () => {
    expect(
      parseRegistryError(new Error('SKILL_REGISTRY_RATE_LIMITED: rate limited: retry after 42s'))
    ).toEqual({
      kind: 'rate_limited',
      message: 'rate limited: retry after 42s',
      retryAfterSecs: 42,
    });
    expect(parseRegistryError(new Error('rpc failed: SKILL_REGISTRY_NOT_FOUND: no entry'))).toEqual(
      { kind: 'not_found', message: 'no entry', retryAfterSecs: null }
    );
    expect(parseRegistryError('boom')).toEqual({
      kind: null,
      message: 'boom',
      retryAfterSecs: null,
    });
  });

  it('isInstallable prefers the core flag over the download url', () => {
    const base = {
      id: 'x',
      name: 'x',
      description: '',
      source: 's',
      category: '',
      author: null,
      version: null,
      tags: [],
      platforms: [],
      docs_path: null,
      commands: [],
      env_vars: [],
      license: null,
    };
    expect(isInstallable({ ...base, download_url: '', installable: true })).toBe(true);
    expect(isInstallable({ ...base, download_url: 'https://e/SKILL.md', installable: false })).toBe(
      false
    );
    expect(isInstallable({ ...base, download_url: 'https://e/SKILL.md' })).toBe(true);
    expect(isInstallable({ ...base, download_url: '' })).toBe(false);
  });
});
