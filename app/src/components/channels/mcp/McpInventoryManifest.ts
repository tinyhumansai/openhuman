/**
 * Sharable MCP Inventory — portable, versioned, secret-free manifest of
 * installed MCP servers.
 *
 * The shape is intentionally NOT just `InstalledServer[]`:
 *
 *   - `server_id` is per-machine (UUID) and would mean nothing on the
 *     importer's host. Stripped on export.
 *   - `installed_at` / `last_connected_at` are local-time observability
 *     fields irrelevant to the importer. Stripped.
 *   - `env` *values* are SECRETS. Only the `env_keys` (NAMES) make the
 *     manifest. The importer fills values per-server.
 *   - `command` / `args` are intentionally NOT carried — the importer's
 *     core decides how to spawn from the upstream registry entry. This
 *     keeps manifests portable across `npx` / `uvx` upgrades and avoids
 *     baking transient command shapes into shared artifacts.
 *
 * The schema field (`$schema`) is a string sentinel rather than a URL
 * so the manifest is fully self-contained and can be validated offline.
 * Bump `CURRENT_MANIFEST_VERSION` if the shape changes.
 */
import type { InstalledServer } from './types';

/**
 * Sentinel embedded in every exported manifest. Importer rejects any
 * payload whose `$schema` does not match exactly.
 */
export const CURRENT_MANIFEST_SCHEMA = 'openhuman.mcp-inventory.v1' as const;

/**
 * Per-server entry in the exported manifest. No secrets, no per-machine
 * identifiers. Optional fields are omitted when absent (NOT serialised as
 * `null` / `undefined`) to keep manifests stable across re-exports.
 */
export interface McpInventoryEntry {
  qualified_name: string;
  display_name: string;
  description?: string;
  /** ENV variable NAMES (not values). The importer collects values. */
  env_keys: string[];
  /** Free-form non-secret config blob the server may need. */
  config?: unknown;
}

export interface McpInventoryManifest {
  $schema: typeof CURRENT_MANIFEST_SCHEMA;
  /** ISO-8601 UTC timestamp captured at export time. */
  exported_at: string;
  /** Free-form label for the exporting environment (host, user, env). */
  exported_by: string;
  servers: McpInventoryEntry[];
}

/**
 * Build the export entry for one installed server. Centralised here so
 * the redaction contract ("no secret values, no per-machine ids") is
 * stated exactly once and tested exactly once.
 */
const toEntry = (server: InstalledServer): McpInventoryEntry => {
  const entry: McpInventoryEntry = {
    qualified_name: server.qualified_name,
    display_name: server.display_name,
    env_keys: Array.isArray(server.env_keys) ? [...server.env_keys].sort() : [],
  };
  if (server.description) entry.description = server.description;
  if (server.config !== undefined && server.config !== null) entry.config = server.config;
  return entry;
};

/** Produce a manifest object from a list of installed servers. */
export function buildManifest(
  servers: InstalledServer[],
  exportedBy = 'openhuman-desktop'
): McpInventoryManifest {
  return {
    $schema: CURRENT_MANIFEST_SCHEMA,
    exported_at: new Date().toISOString(),
    exported_by: exportedBy,
    // Sort by qualified_name for deterministic output (re-exporting the
    // same set twice produces byte-identical manifests, which makes
    // them diff-friendly in source control).
    servers: servers.map(toEntry).sort((a, b) => a.qualified_name.localeCompare(b.qualified_name)),
  };
}

/** Pretty-print a manifest to JSON suitable for clipboard / download. */
export function serializeManifest(manifest: McpInventoryManifest): string {
  return `${JSON.stringify(manifest, null, 2)}\n`;
}

/**
 * Discriminated-union result of parsing a manifest. Errors carry a
 * single user-facing message and (when applicable) the path to the
 * first offending field — surfaced as-is in the import UI's alert.
 */
export type ParseResult =
  | { ok: true; manifest: McpInventoryManifest }
  | { ok: false; error: string };

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const isStringArray = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every(v => typeof v === 'string');

/**
 * Parse + validate a raw manifest string. Returns a discriminated union
 * with a single message on failure — never throws. Tolerant of trailing
 * whitespace; strict on the rest.
 */
export function parseManifest(raw: string): ParseResult {
  if (typeof raw !== 'string' || raw.trim().length === 0) {
    return { ok: false, error: 'Manifest is empty.' };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (err) {
    const detail = err instanceof Error ? err.message : 'JSON parse failed.';
    return { ok: false, error: `Invalid JSON: ${detail}` };
  }
  if (!isObject(parsed)) {
    return { ok: false, error: 'Manifest must be a JSON object at the root.' };
  }
  if (parsed.$schema !== CURRENT_MANIFEST_SCHEMA) {
    return {
      ok: false,
      error: `Unsupported manifest schema: expected "${CURRENT_MANIFEST_SCHEMA}", got "${String(
        parsed.$schema
      )}".`,
    };
  }
  if (typeof parsed.exported_at !== 'string' || parsed.exported_at.length === 0) {
    return { ok: false, error: 'Missing or invalid `exported_at`.' };
  }
  if (typeof parsed.exported_by !== 'string') {
    return { ok: false, error: 'Missing or invalid `exported_by`.' };
  }
  if (!Array.isArray(parsed.servers)) {
    return { ok: false, error: 'Missing or invalid `servers` array.' };
  }
  const servers: McpInventoryEntry[] = [];
  for (let i = 0; i < parsed.servers.length; i += 1) {
    const raw = parsed.servers[i];
    if (!isObject(raw)) {
      return { ok: false, error: `servers[${i}] is not an object.` };
    }
    if (typeof raw.qualified_name !== 'string' || raw.qualified_name.length === 0) {
      return { ok: false, error: `servers[${i}].qualified_name is missing or empty.` };
    }
    if (typeof raw.display_name !== 'string' || raw.display_name.length === 0) {
      return { ok: false, error: `servers[${i}].display_name is missing or empty.` };
    }
    if (!isStringArray(raw.env_keys)) {
      return { ok: false, error: `servers[${i}].env_keys must be an array of strings.` };
    }
    // Pre-import safety net — refuse manifests that smuggle in an `env`
    // map. (The exporter never writes one, but an attacker / leaked
    // file might. We want NO path where parseManifest hands the
    // importer concrete secret values.)
    if ('env' in raw) {
      return {
        ok: false,
        error: `servers[${i}] contains an "env" field with secret values. Refusing to import; manifests must only carry env_keys (names).`,
      };
    }
    const entry: McpInventoryEntry = {
      qualified_name: raw.qualified_name,
      display_name: raw.display_name,
      env_keys: raw.env_keys,
    };
    if (typeof raw.description === 'string') entry.description = raw.description;
    if ('config' in raw && raw.config !== undefined && raw.config !== null) {
      entry.config = raw.config;
    }
    servers.push(entry);
  }
  return {
    ok: true,
    manifest: {
      $schema: CURRENT_MANIFEST_SCHEMA,
      exported_at: parsed.exported_at,
      exported_by: parsed.exported_by,
      servers,
    },
  };
}

/**
 * Per-entry import classification. The Import UI uses these statuses to
 * colour-code the preview table and decide whether to surface an
 * "Install" action.
 */
export type ImportEntryStatus = 'new' | 'already_installed';

export interface ClassifiedImportEntry {
  entry: McpInventoryEntry;
  status: ImportEntryStatus;
}

/**
 * Cross-reference each manifest entry against the importer's current
 * installed servers (by `qualified_name`) to classify what would happen
 * on install. Stable order: matches the manifest's input order.
 */
export function classifyImport(
  manifest: McpInventoryManifest,
  installed: InstalledServer[]
): ClassifiedImportEntry[] {
  const installedNames = new Set(installed.map(s => s.qualified_name));
  return manifest.servers.map(entry => ({
    entry,
    status: installedNames.has(entry.qualified_name) ? 'already_installed' : 'new',
  }));
}

/** Suggested default filename for browser-side download. */
export function suggestedFilename(manifest: McpInventoryManifest): string {
  // exported_at is "2026-05-25T20:14:15.123Z"; trim to a filename-safe
  // YYYYMMDDHHMMSS slug so the file sorts well in directory listings.
  const stamp = manifest.exported_at.replace(/[-:T]/g, '').replace(/\..*$/, '');
  return `openhuman-mcp-inventory-${stamp}.json`;
}
