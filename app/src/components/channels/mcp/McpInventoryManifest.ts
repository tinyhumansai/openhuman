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
 * Stable, locale-independent identifiers for every parse-failure mode.
 *
 * Mapped to translated text by the consumer (UI) via
 * `mcp.inventory.parseError.<code>` i18n keys — so the manifest layer
 * stays decoupled from any presentation locale, and the failure modes
 * are a fixed contract that external tooling (CLIs, test fixtures,
 * other clients) can match on without depending on the rendered text.
 */
export type ParseErrorCode =
  | 'empty'
  | 'invalidJson'
  | 'rootNotObject'
  | 'unsupportedSchema'
  | 'missingExportedAt'
  | 'missingExportedBy'
  | 'invalidServers'
  | 'serverNotObject'
  | 'serverMissingQualifiedName'
  | 'serverMissingDisplayName'
  | 'serverEnvKeysNotArray'
  | 'serverContainsEnv'
  | 'duplicateQualifiedName';

/**
 * Discriminated-union result of parsing a manifest. On failure carries
 * a stable `errorCode` for i18n + an optional `detail` string with
 * machine context (JSON parse exception text, offending index, the
 * actual schema string we got, etc.). Consumers render via
 * `t(\`mcp.inventory.parseError.\${errorCode}\`)` + optional detail.
 */
export type ParseResult =
  | { ok: true; manifest: McpInventoryManifest }
  | { ok: false; errorCode: ParseErrorCode; detail?: string };

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const isStringArray = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every(v => typeof v === 'string');

/**
 * Parse + validate a raw manifest string. Returns a discriminated union
 * with a stable `errorCode` on failure — never throws. Tolerant of
 * trailing whitespace; strict on the rest. Includes a duplicate-
 * `qualified_name` check so a malformed/malicious manifest can't
 * quietly install the same server twice with diverging env_keys.
 */
export function parseManifest(raw: string): ParseResult {
  if (typeof raw !== 'string' || raw.trim().length === 0) {
    return { ok: false, errorCode: 'empty' };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (err) {
    return {
      ok: false,
      errorCode: 'invalidJson',
      detail: err instanceof Error ? err.message : undefined,
    };
  }
  if (!isObject(parsed)) {
    return { ok: false, errorCode: 'rootNotObject' };
  }
  if (parsed.$schema !== CURRENT_MANIFEST_SCHEMA) {
    return {
      ok: false,
      errorCode: 'unsupportedSchema',
      detail: `expected "${CURRENT_MANIFEST_SCHEMA}", got "${String(parsed.$schema)}"`,
    };
  }
  if (typeof parsed.exported_at !== 'string' || parsed.exported_at.length === 0) {
    return { ok: false, errorCode: 'missingExportedAt' };
  }
  if (typeof parsed.exported_by !== 'string' || parsed.exported_by.trim().length === 0) {
    // Blank/whitespace-only `exported_by` is treated as missing: the field
    // is observability metadata (which host/user produced the manifest)
    // and an empty value would render as "Exported from " in the preview.
    return { ok: false, errorCode: 'missingExportedBy' };
  }
  if (!Array.isArray(parsed.servers)) {
    return { ok: false, errorCode: 'invalidServers' };
  }
  const servers: McpInventoryEntry[] = [];
  // Track qualified_names we've already accepted, so a manifest that
  // lists the same server twice (with possibly diverging env_keys or
  // config) is rejected up-front rather than silently producing two
  // import rows that would both call install on the same upstream id.
  const seenQualifiedNames = new Set<string>();
  for (let i = 0; i < parsed.servers.length; i += 1) {
    const raw = parsed.servers[i];
    if (!isObject(raw)) {
      return { ok: false, errorCode: 'serverNotObject', detail: `servers[${i}]` };
    }
    if (typeof raw.qualified_name !== 'string' || raw.qualified_name.length === 0) {
      return { ok: false, errorCode: 'serverMissingQualifiedName', detail: `servers[${i}]` };
    }
    if (seenQualifiedNames.has(raw.qualified_name)) {
      return {
        ok: false,
        errorCode: 'duplicateQualifiedName',
        detail: `servers[${i}]: "${raw.qualified_name}"`,
      };
    }
    seenQualifiedNames.add(raw.qualified_name);
    if (typeof raw.display_name !== 'string' || raw.display_name.length === 0) {
      return { ok: false, errorCode: 'serverMissingDisplayName', detail: `servers[${i}]` };
    }
    if (!isStringArray(raw.env_keys)) {
      return { ok: false, errorCode: 'serverEnvKeysNotArray', detail: `servers[${i}]` };
    }
    // Pre-import safety net — refuse manifests that smuggle in an `env`
    // map. (The exporter never writes one, but an attacker / leaked
    // file might. We want NO path where parseManifest hands the
    // importer concrete secret values.)
    if ('env' in raw) {
      return { ok: false, errorCode: 'serverContainsEnv', detail: `servers[${i}]` };
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
  //
  // The character class is built from a String.fromCharCode array rather
  // than a literal regex character class because Tailwind v4's content
  // scanner greps every JS/TS source string for arbitrary-value class
  // shapes and would mis-identify the literal as a Tailwind class,
  // emitting an invalid CSS rule that fails `lightningcss minify` during
  // the Tauri production build.
  const SEPARATORS = String.fromCharCode(45, 58, 84); // '-', ':', 'T'
  const stripPattern = new RegExp('[' + SEPARATORS + ']', 'g');
  const stamp = manifest.exported_at.replace(stripPattern, '').replace(/\..*$/, '');
  return `openhuman-mcp-inventory-${stamp}.json`;
}
