import { beforeEach, describe, expect, it, vi } from 'vitest';

import { declareServer, isAddCandidate, resolveHostedEntry } from './registryAdd';
import type { SmitheryConnection, SmitheryServer, SmitheryServerDetail } from './types';

const mockConfigGet = vi.fn();
const mockConfigSet = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    configGet: (...args: unknown[]) => mockConfigGet(...args),
    configSet: (...args: unknown[]) => mockConfigSet(...args),
  },
}));

const SERVER: SmitheryServer = {
  qualified_name: 'com.example/open',
  display_name: 'Open',
  is_deployed: true,
  source: 'mcp_official',
};

const http = (deployment_url: string, extra: Partial<SmitheryConnection> = {}) =>
  ({ type: 'http', deployment_url, ...extra }) as SmitheryConnection;

const detail = (
  connections: SmitheryConnection[],
  extra: Partial<SmitheryServerDetail> = {}
): SmitheryServerDetail => ({ ...SERVER, connections, required_env_keys: [], ...extra });

describe('isAddCandidate', () => {
  it('offers Add only for hosted, non-Smithery, non-API-key rows', () => {
    expect(isAddCandidate(SERVER)).toBe(true);
    expect(isAddCandidate({ ...SERVER, source: 'smithery' })).toBe(false);
    expect(isAddCandidate({ ...SERVER, auth_kind: 'api_key' })).toBe(false);
    expect(isAddCandidate({ ...SERVER, is_deployed: false })).toBe(false);
    expect(isAddCandidate({ ...SERVER, is_deployed: undefined })).toBe(false);
  });
});

describe('resolveHostedEntry', () => {
  it('declares the one hosted endpoint, with no headers or type', () => {
    const result = resolveHostedEntry(
      detail([http('https://mcp.example.com/mcp'), { type: 'stdio' }]),
      SERVER
    );
    expect(result).toEqual({ ok: true, entry: { url: 'https://mcp.example.com/mcp' } });
  });

  it('accepts sse connections and repeats of the same url', () => {
    const sse = { type: 'sse', deployment_url: 'https://mcp.example.com/sse' } as unknown;
    const result = resolveHostedEntry(
      detail([sse as SmitheryConnection, sse as SmitheryConnection]),
      SERVER
    );
    expect(result).toEqual({ ok: true, entry: { url: 'https://mcp.example.com/sse' } });
  });

  it('passes the description through', () => {
    expect(
      resolveHostedEntry(detail([http('https://a.example/mcp')]), {
        ...SERVER,
        description: '  Does things.  ',
      })
    ).toEqual({ ok: true, entry: { url: 'https://a.example/mcp', description: 'Does things.' } });
    expect(
      resolveHostedEntry(detail([http('https://a.example/mcp')], { description: 'From detail' }), {
        ...SERVER,
      })
    ).toEqual({ ok: true, entry: { url: 'https://a.example/mcp', description: 'From detail' } });
  });

  it.each([
    ['a stdio-only server', detail([{ type: 'stdio' }]), 'no_hosted_connection'],
    ['no connections at all', { ...SERVER } as SmitheryServerDetail, 'no_hosted_connection'],
    ['a hosted connection without a url', detail([{ type: 'http' }]), 'missing_url'],
    ['a templated url', detail([http('https://{tenant}.example.com/mcp')]), 'templated_url'],
    ['an unparsable url', detail([http('not a url')]), 'invalid_url'],
    ['a non-http scheme', detail([http('ws://mcp.example.com')]), 'unsupported_scheme'],
    [
      'more than one distinct url',
      detail([http('https://a.example/mcp'), http('https://b.example/mcp')]),
      'multiple_urls',
    ],
    [
      'a required schema input',
      detail([
        http('https://a.example/mcp', {
          config_schema: { properties: { region: {} }, required: ['region'] },
        }),
      ]),
      'requires_input',
    ],
    [
      'a secret schema property',
      detail([
        http('https://a.example/mcp', {
          config_schema: { properties: { Authorization: { 'x-secret': true } } },
        }),
      ]),
      'secret_input',
    ],
    [
      'an isSecret schema property',
      detail([
        http('https://a.example/mcp', {
          config_schema: { properties: { token: { isSecret: true }, note: 'x' } },
        }),
      ]),
      'secret_input',
    ],
    [
      'required env keys',
      detail([http('https://a.example/mcp')], { required_env_keys: ['API_KEY'] }),
      'requires_env',
    ],
  ])('refuses %s', (_label, input, reason) => {
    expect(resolveHostedEntry(input as SmitheryServerDetail, SERVER)).toEqual({
      ok: false,
      reason,
    });
  });

  it('accepts optional, non-secret schema properties', () => {
    const result = resolveHostedEntry(
      detail([
        http('https://a.example/mcp', {
          config_schema: { properties: { logLevel: { description: 'verbosity' } } },
        }),
      ]),
      SERVER
    );
    expect(result.ok).toBe(true);
  });
});

describe('declareServer', () => {
  beforeEach(() => {
    mockConfigGet.mockReset();
    mockConfigSet.mockReset();
  });

  it('leaves an existing entry alone and writes nothing', async () => {
    mockConfigGet.mockResolvedValue({
      mcpServers: { 'com.example/open': { url: 'https://old.example' } },
    });
    await expect(declareServer('com.example/open', { url: 'https://new.example' })).resolves.toBe(
      'exists'
    );
    expect(mockConfigSet).not.toHaveBeenCalled();
  });

  it('adds the entry and keeps every other one', async () => {
    mockConfigGet.mockResolvedValue({
      mcpServers: { echo: { command: 'npx', args: ['-y', 'echo'], envKeys: ['K'] } },
    });
    mockConfigSet.mockResolvedValue({ mcpServers: {}, added: [], updated: [], removed: [] });
    await expect(declareServer('com.example/open', { url: 'https://a.example' })).resolves.toBe(
      'added'
    );
    expect(mockConfigSet).toHaveBeenCalledWith({
      mcpServers: {
        echo: { command: 'npx', args: ['-y', 'echo'], envKeys: ['K'] },
        'com.example/open': { url: 'https://a.example' },
      },
    });
  });

  it('treats a missing mcpServers map as empty', async () => {
    mockConfigGet.mockResolvedValue({});
    mockConfigSet.mockResolvedValue({ mcpServers: {}, added: [], updated: [], removed: [] });
    await expect(declareServer('x', { url: 'https://a.example' })).resolves.toBe('added');
    expect(mockConfigSet).toHaveBeenCalledWith({ mcpServers: { x: { url: 'https://a.example' } } });
  });

  it('rethrows a core refusal verbatim', async () => {
    mockConfigGet.mockResolvedValue({ mcpServers: {} });
    const refusal = new Error('`x` needs a `url` (hosted) or a `command` (run locally)');
    mockConfigSet.mockRejectedValue(refusal);
    await expect(declareServer('x', { url: 'https://a.example' })).rejects.toBe(refusal);
  });
});
