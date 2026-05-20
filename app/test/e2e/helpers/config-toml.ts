/**
 * Read-only helpers for the core's on-disk `config.toml`.
 *
 * `app/scripts/e2e-run-session.sh` always exports `OPENHUMAN_WORKSPACE` and
 * the core writes its config under that root (see `Config::load_or_init`).
 * We assert against the resulting file in onboarding/settings specs that
 * need to confirm UI mutations land in the persisted config.
 *
 * Intentionally minimal: a regex-based reader, not a full TOML parser. The
 * keys we care about (`onboarding_completed`, `voice_server.auto_start`,
 * etc.) are simple `key = value` lines at the top-level or inside a flat
 * `[section]` table.
 */
import fs from 'node:fs';
import path from 'node:path';

function workspaceRoot(): string {
  const ws = process.env.OPENHUMAN_WORKSPACE?.trim();
  if (ws && ws.length > 0) return ws;
  const home = process.env.HOME || '';
  return path.join(home, '.openhuman');
}

export function configTomlPath(): string {
  return path.join(workspaceRoot(), 'config.toml');
}

export function readConfigToml(): string {
  const file = configTomlPath();
  if (!fs.existsSync(file)) {
    throw new Error(`[config-toml] expected config.toml at ${file}`);
  }
  return fs.readFileSync(file, 'utf8');
}

/**
 * Extract a top-level `key = value` line. Returns the raw RHS (quotes
 * included for strings) or null if the key isn't present.
 */
export function topLevelValue(contents: string, key: string): string | null {
  const lines = contents.split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith('[')) {
      // Stop at the first section header — we only scan the top-level block.
      break;
    }
    const m = trimmed.match(/^([A-Za-z0-9_]+)\s*=\s*(.+)$/);
    if (m && m[1] === key) {
      return m[2].trim();
    }
  }
  return null;
}

/**
 * Extract a `key = value` line inside `[section]`. Works for any flat
 * `[a.b]`-style section name.
 */
export function sectionValue(contents: string, section: string, key: string): string | null {
  const lines = contents.split(/\r?\n/);
  let inSection = false;
  const target = `[${section}]`;
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith('[')) {
      inSection = trimmed === target;
      continue;
    }
    if (!inSection) continue;
    const m = trimmed.match(/^([A-Za-z0-9_]+)\s*=\s*(.+)$/);
    if (m && m[1] === key) {
      return m[2].trim();
    }
  }
  return null;
}

export function readBool(raw: string | null): boolean | null {
  if (raw === null) return null;
  if (raw === 'true') return true;
  if (raw === 'false') return false;
  return null;
}

/** Convenience: read a string-valued key from a section, stripping `"`. */
export function readSectionString(contents: string, section: string, key: string): string | null {
  const raw = sectionValue(contents, section, key);
  if (raw === null) return null;
  const m = raw.match(/^"((?:[^"\\]|\\.)*)"$/);
  return m ? m[1] : raw;
}
