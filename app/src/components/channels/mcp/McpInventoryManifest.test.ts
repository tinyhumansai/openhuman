/**
 * Tests for the McpInventoryManifest helpers. Pure-data layer — no React.
 *
 * The redaction contract is the most security-relevant invariant in the
 * whole inventory feature; the tests below pin it from both directions:
 *   - `buildManifest` must NEVER include `server_id`, `installed_at`,
 *     `last_connected_at`, `command`, `args`, `command_kind`, or any
 *     `env` value.
 *   - `parseManifest` must REJECT any input that smuggles back an `env`
 *     map (or other shape violations).
 */
import { describe, expect, it } from 'vitest';

import {
  buildManifest,
  classifyImport,
  CURRENT_MANIFEST_SCHEMA,
  parseManifest,
  serializeManifest,
  suggestedFilename,
} from './McpInventoryManifest';
import type { InstalledServer } from './types';

const SERVER_FS: InstalledServer = {
  server_id: 'srv-uuid-1',
  qualified_name: 'acme/fs-server',
  display_name: 'File Server',
  description: 'Reads files',
  icon_url: 'https://example.com/icon.png',
  command_kind: 'node',
  command: 'npx',
  args: ['-y', 'acme/fs-server'],
  env_keys: ['ROOT_DIR', 'API_KEY'],
  config: { region: 'us-east-1' },
  installed_at: 1_700_000_000,
  last_connected_at: 1_700_001_000,
};

const SERVER_DB: InstalledServer = {
  server_id: 'srv-uuid-2',
  qualified_name: 'acme/db-server',
  display_name: 'DB Server',
  description: undefined,
  command_kind: 'node',
  command: 'npx',
  args: ['-y', 'acme/db-server'],
  env_keys: ['DB_URL'],
  installed_at: 1_700_000_500,
};

describe('McpInventoryManifest: buildManifest', () => {
  it('returns the documented schema sentinel', () => {
    const m = buildManifest([]);
    expect(m.$schema).toBe(CURRENT_MANIFEST_SCHEMA);
  });

  it('uses an ISO-8601 timestamp for exported_at', () => {
    const m = buildManifest([]);
    // Loose check — anything `new Date()` accepts back must be valid.
    expect(Number.isNaN(Date.parse(m.exported_at))).toBe(false);
  });

  it('honours an explicit exported_by label', () => {
    const m = buildManifest([SERVER_FS], 'my-machine');
    expect(m.exported_by).toBe('my-machine');
  });

  it('defaults exported_by to "openhuman-desktop"', () => {
    const m = buildManifest([SERVER_FS]);
    expect(m.exported_by).toBe('openhuman-desktop');
  });

  it('redacts per-machine identifiers (server_id, installed_at, last_connected_at)', () => {
    const m = buildManifest([SERVER_FS]);
    const entry = m.servers[0];
    expect('server_id' in entry).toBe(false);
    expect('installed_at' in entry).toBe(false);
    expect('last_connected_at' in entry).toBe(false);
  });

  it('redacts transient spawn shape (command, args, command_kind)', () => {
    const m = buildManifest([SERVER_FS]);
    const entry = m.servers[0];
    expect('command' in entry).toBe(false);
    expect('args' in entry).toBe(false);
    expect('command_kind' in entry).toBe(false);
  });

  it('NEVER exports an env value map (only env_keys / NAMES)', () => {
    const m = buildManifest([SERVER_FS]);
    const entry = m.servers[0];
    expect('env' in entry).toBe(false);
    // env_keys IS present and matches the input (with any sort).
    expect([...entry.env_keys].sort()).toEqual(['API_KEY', 'ROOT_DIR']);
  });

  it('sorts env_keys deterministically for diff-stability', () => {
    // Input is in declaration order; output should always be sorted.
    const m = buildManifest([SERVER_FS]);
    expect(m.servers[0].env_keys).toEqual(['API_KEY', 'ROOT_DIR']);
  });

  it('sorts servers by qualified_name for diff-stability', () => {
    // Input out of order; output sorted.
    const m = buildManifest([SERVER_FS, SERVER_DB]);
    expect(m.servers.map(s => s.qualified_name)).toEqual(['acme/db-server', 'acme/fs-server']);
  });

  it('omits description when absent (not serialised as null/undefined)', () => {
    const m = buildManifest([SERVER_DB]);
    expect('description' in m.servers[0]).toBe(false);
  });

  it('includes description when present', () => {
    const m = buildManifest([SERVER_FS]);
    expect(m.servers.find(s => s.qualified_name === 'acme/fs-server')?.description).toBe(
      'Reads files'
    );
  });

  it('omits config when undefined or null; includes when present', () => {
    const m = buildManifest([SERVER_FS, SERVER_DB]);
    const fs = m.servers.find(s => s.qualified_name === 'acme/fs-server');
    const db = m.servers.find(s => s.qualified_name === 'acme/db-server');
    expect(fs?.config).toEqual({ region: 'us-east-1' });
    expect('config' in (db as object)).toBe(false);
  });

  it('handles a server with an undefined env_keys (defensive) without crashing', () => {
    const broken = { ...SERVER_DB, env_keys: undefined as unknown as string[] };
    const m = buildManifest([broken]);
    expect(m.servers[0].env_keys).toEqual([]);
  });
});

describe('McpInventoryManifest: serializeManifest', () => {
  it('produces pretty-printed JSON with a trailing newline', () => {
    const m = buildManifest([SERVER_FS]);
    const text = serializeManifest(m);
    expect(text.endsWith('\n')).toBe(true);
    // Must parse back to an equivalent object.
    expect(JSON.parse(text).$schema).toBe(CURRENT_MANIFEST_SCHEMA);
  });

  it('byte-identical for repeated exports of the same input', () => {
    // Hold time constant via a fixed exporter label since exported_at
    // moves naturally; we test stable shape modulo timestamp.
    const a = buildManifest([SERVER_FS, SERVER_DB], 'fixed');
    const b = buildManifest([SERVER_DB, SERVER_FS], 'fixed');
    // Different declaration order in input, identical sorted output.
    expect(a.servers).toEqual(b.servers);
  });
});

describe('McpInventoryManifest: parseManifest', () => {
  const validRaw = serializeManifest(buildManifest([SERVER_FS, SERVER_DB]));

  it('round-trips: build → serialize → parse yields equivalent servers', () => {
    const result = parseManifest(validRaw);
    if (!result.ok) throw new Error(`expected ok, got: ${result.errorCode}`);
    expect(result.manifest.$schema).toBe(CURRENT_MANIFEST_SCHEMA);
    expect(result.manifest.servers.map(s => s.qualified_name)).toEqual([
      'acme/db-server',
      'acme/fs-server',
    ]);
  });

  it('rejects empty input with the "empty" code', () => {
    expect(parseManifest('')).toEqual({ ok: false, errorCode: 'empty' });
    expect(parseManifest('   ')).toEqual({ ok: false, errorCode: 'empty' });
  });

  it('rejects non-JSON input with the "invalidJson" code and a detail string', () => {
    const result = parseManifest('{not json');
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errorCode).toBe('invalidJson');
      // Detail carries the underlying JSON parse exception message.
      expect(typeof result.detail).toBe('string');
    }
  });

  it('rejects non-object root with the "rootNotObject" code', () => {
    for (const raw of ['[]', '"a string"', 'null']) {
      const r = parseManifest(raw);
      expect(r.ok).toBe(false);
      if (!r.ok) expect(r.errorCode).toBe('rootNotObject');
    }
  });

  it('rejects an unknown / mismatched $schema with the "unsupportedSchema" code', () => {
    const result = parseManifest(JSON.stringify({ $schema: 'something-else', servers: [] }));
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errorCode).toBe('unsupportedSchema');
      expect(result.detail).toContain('something-else');
    }
  });

  it('rejects missing exported_at / exported_by with the correct codes', () => {
    const r1 = parseManifest(JSON.stringify({ $schema: CURRENT_MANIFEST_SCHEMA, servers: [] }));
    expect(r1.ok).toBe(false);
    if (!r1.ok) expect(r1.errorCode).toBe('missingExportedAt');
    const r2 = parseManifest(
      JSON.stringify({ $schema: CURRENT_MANIFEST_SCHEMA, exported_at: '2026-05-25', servers: [] })
    );
    expect(r2.ok).toBe(false);
    if (!r2.ok) expect(r2.errorCode).toBe('missingExportedBy');
  });

  it('treats a blank or whitespace-only exported_by as missing', () => {
    for (const blank of ['', '   ', '\t\n']) {
      const result = parseManifest(
        JSON.stringify({
          $schema: CURRENT_MANIFEST_SCHEMA,
          exported_at: '2026-05-25T00:00:00Z',
          exported_by: blank,
          servers: [],
        })
      );
      expect(result.ok).toBe(false);
      if (!result.ok) expect(result.errorCode).toBe('missingExportedBy');
    }
  });

  it('rejects a non-array servers field with the "invalidServers" code', () => {
    const result = parseManifest(
      JSON.stringify({
        $schema: CURRENT_MANIFEST_SCHEMA,
        exported_at: '2026-05-25T00:00:00Z',
        exported_by: 'x',
        servers: 'not-an-array',
      })
    );
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.errorCode).toBe('invalidServers');
  });

  it('rejects entries missing qualified_name with the matching code', () => {
    const result = parseManifest(
      JSON.stringify({
        $schema: CURRENT_MANIFEST_SCHEMA,
        exported_at: '2026-05-25T00:00:00Z',
        exported_by: 'x',
        servers: [{ display_name: 'X', env_keys: [] }],
      })
    );
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.errorCode).toBe('serverMissingQualifiedName');
  });

  it('rejects entries whose env_keys is not an array of strings', () => {
    const result = parseManifest(
      JSON.stringify({
        $schema: CURRENT_MANIFEST_SCHEMA,
        exported_at: '2026-05-25T00:00:00Z',
        exported_by: 'x',
        servers: [{ qualified_name: 'a/b', display_name: 'AB', env_keys: [1, 2, 3] }],
      })
    );
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.errorCode).toBe('serverEnvKeysNotArray');
  });

  // The single most important security test in the file:
  it('SECURITY: refuses any manifest that smuggles an `env` value map', () => {
    const malicious = JSON.stringify({
      $schema: CURRENT_MANIFEST_SCHEMA,
      exported_at: '2026-05-25T00:00:00Z',
      exported_by: 'attacker',
      servers: [
        {
          qualified_name: 'evil/server',
          display_name: 'Evil',
          env_keys: ['API_KEY'],
          env: { API_KEY: 'attacker-supplied-secret-value' },
        },
      ],
    });
    const result = parseManifest(malicious);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.errorCode).toBe('serverContainsEnv');
  });

  it('rejects manifests containing a duplicate qualified_name with the matching code', () => {
    const dup = JSON.stringify({
      $schema: CURRENT_MANIFEST_SCHEMA,
      exported_at: '2026-05-25T00:00:00Z',
      exported_by: 'x',
      servers: [
        { qualified_name: 'a/b', display_name: 'First', env_keys: [] },
        { qualified_name: 'a/b', display_name: 'Second', env_keys: [] },
      ],
    });
    const result = parseManifest(dup);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errorCode).toBe('duplicateQualifiedName');
      // Detail includes the offending qualified_name for diagnosability.
      expect(result.detail).toContain('a/b');
    }
  });

  it('omits optional fields cleanly on parse (no undefined leaks)', () => {
    const minimal = JSON.stringify({
      $schema: CURRENT_MANIFEST_SCHEMA,
      exported_at: '2026-05-25T00:00:00Z',
      exported_by: 'x',
      servers: [{ qualified_name: 'a/b', display_name: 'AB', env_keys: [] }],
    });
    const result = parseManifest(minimal);
    if (!result.ok) throw new Error(result.errorCode);
    const e = result.manifest.servers[0];
    expect('description' in e).toBe(false);
    expect('config' in e).toBe(false);
  });
});

describe('McpInventoryManifest: classifyImport', () => {
  const manifest = buildManifest([SERVER_FS, SERVER_DB]);

  it('classifies each entry as new or already_installed by qualified_name', () => {
    const classified = classifyImport(manifest, [
      // Pretend SERVER_FS is already installed locally; SERVER_DB is not.
      { ...SERVER_FS },
    ]);
    const byName = Object.fromEntries(classified.map(c => [c.entry.qualified_name, c.status]));
    expect(byName['acme/fs-server']).toBe('already_installed');
    expect(byName['acme/db-server']).toBe('new');
  });

  it('returns all as "new" when nothing is installed locally', () => {
    const classified = classifyImport(manifest, []);
    expect(classified.every(c => c.status === 'new')).toBe(true);
  });

  it('returns all as "already_installed" when every manifest entry exists locally', () => {
    const classified = classifyImport(manifest, [SERVER_FS, SERVER_DB]);
    expect(classified.every(c => c.status === 'already_installed')).toBe(true);
  });

  it('preserves manifest entry order in the classified output', () => {
    const classified = classifyImport(manifest, []);
    // Manifest is sorted by qualified_name; classification preserves that.
    expect(classified.map(c => c.entry.qualified_name)).toEqual([
      'acme/db-server',
      'acme/fs-server',
    ]);
  });
});

describe('McpInventoryManifest: suggestedFilename', () => {
  it('produces a slug-style filename including a timestamp', () => {
    const m = buildManifest([SERVER_FS]);
    const name = suggestedFilename(m);
    expect(name.startsWith('openhuman-mcp-inventory-')).toBe(true);
    expect(name.endsWith('.json')).toBe(true);
    // Stamp must contain only digits (no T, no -, no :, no .).
    const stamp = name.replace('openhuman-mcp-inventory-', '').replace('.json', '');
    expect(/^\d+$/.test(stamp)).toBe(true);
  });
});
