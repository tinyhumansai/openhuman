/**
 * Client for the GitHub-published mascot manifest (`dist/mascots.json`).
 *
 * The manifest and its `.riv` runtime files are fetched directly from
 * `raw.githubusercontent.com` (CORS-open, allowed by the webview CSP). Three
 * cache layers keep the picker and Human stage snappy and offline-tolerant:
 *
 *   1. an in-memory promise (one network fetch per session, even under
 *      concurrent callers),
 *   2. a `localStorage` snapshot (survives reloads / brief offline), and
 *   3. the IndexedDB `.riv` binary cache (`rivCache`, keyed by sha256) so
 *      switching back to a previously-seen mascot never re-downloads the asset.
 */
import debug from 'debug';

import { MASCOT_MANIFEST_URL } from '../../../../utils/config';
import { loadRivBuffer } from '../rivCache';
import {
  type MascotManifest,
  type MascotManifestChannel,
  type MascotManifestEntry,
  type MascotManifestFile,
  runtimeFile,
} from './types';

const log = debug('human:mascot:manifest');

/** localStorage key for the last successfully-fetched manifest snapshot. */
const SNAPSHOT_KEY = 'openhuman.mascotManifest.v1';
const MANIFEST_FETCH_TIMEOUT_MS = 5_000;

/** Session-lifetime in-flight/resolved fetch. Shared across all callers. */
let inflight: Promise<MascotManifest> | null = null;

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0;
}

function isManifestFile(value: unknown): value is MascotManifestFile {
  const f = value as Partial<MascotManifestFile> | null;
  return (
    !!f &&
    isNonEmptyString(f.path) &&
    isNonEmptyString(f.url) &&
    isNonEmptyString(f.sha256) &&
    (f.role === 'runtime' || f.role === 'source')
  );
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.length > 0 && value.every(isNonEmptyString);
}

function isManifestChannel(value: unknown): value is MascotManifestChannel {
  const channel = value as Partial<MascotManifestChannel> | null;
  if (!channel || !isNonEmptyString(channel.key) || !isStringArray(channel.values)) return false;
  if (channel.enum !== undefined && !isNonEmptyString(channel.enum)) return false;
  if (channel.label !== undefined && !isNonEmptyString(channel.label)) return false;
  if (channel.default !== undefined && !isNonEmptyString(channel.default)) return false;
  if (channel.cycle !== undefined) {
    const cycle = channel.cycle;
    if (typeof cycle.enabled !== 'boolean') return false;
    if (
      cycle.intervalMs !== undefined &&
      (!Number.isFinite(cycle.intervalMs) || cycle.intervalMs <= 0)
    ) {
      return false;
    }
    if (cycle.order !== undefined && cycle.order !== 'random' && cycle.order !== 'sequential') {
      return false;
    }
  }
  return true;
}

function isManifestEntry(value: unknown): value is MascotManifestEntry {
  const m = value as Partial<MascotManifestEntry> | null;
  if (!m || !isNonEmptyString(m.id) || !isNonEmptyString(m.name)) return false;
  if (m.status !== 'ready' && m.status !== 'draft') return false;
  const se = m.stateEngine;
  if (
    !se ||
    !isStringArray(se.visemeCodes) ||
    !isStringArray(se.idlePoseCycle) ||
    !se.states ||
    !isNonEmptyString(se.states.idle) ||
    !isNonEmptyString(se.states.thinking)
  ) {
    return false;
  }
  if (!Object.values(se.states).every(isNonEmptyString)) return false;
  if (
    se.channels !== undefined &&
    (!Array.isArray(se.channels) || !se.channels.every(isManifestChannel))
  ) {
    return false;
  }
  if (!Array.isArray(m.files) || !m.files.every(isManifestFile)) return false;
  // A renderable mascot needs a runtime `.riv`; drop entries that only ship a
  // source `.rev` so callers never select an unplayable asset.
  return !!runtimeFile(m as MascotManifestEntry);
}

/**
 * Validate and normalise a parsed manifest document. Throws on a
 * fundamentally malformed payload; silently drops individual malformed
 * mascot entries (a single bad entry should never blank the whole library).
 */
export function parseManifest(raw: unknown): MascotManifest {
  const doc = raw as Partial<MascotManifest> | null;
  if (!doc || !Array.isArray(doc.mascots)) {
    throw new Error('mascot manifest: missing mascots array');
  }
  const mascots = doc.mascots.filter(isManifestEntry);
  if (mascots.length === 0) {
    throw new Error('mascot manifest: no renderable mascots');
  }
  return {
    schemaVersion: typeof doc.schemaVersion === 'number' ? doc.schemaVersion : 1,
    generatedAt: isNonEmptyString(doc.generatedAt) ? doc.generatedAt : '',
    mascots,
    source: {
      repository: doc.source?.repository ?? '',
      branch: doc.source?.branch ?? '',
      commit: doc.source?.commit ?? '',
    },
  };
}

function readSnapshot(): MascotManifest | null {
  try {
    const raw = window.localStorage.getItem(SNAPSHOT_KEY);
    if (!raw) return null;
    return parseManifest(JSON.parse(raw));
  } catch (err) {
    log('snapshot read failed: %o', err);
    return null;
  }
}

function writeSnapshot(manifest: MascotManifest): void {
  try {
    window.localStorage.setItem(SNAPSHOT_KEY, JSON.stringify(manifest));
  } catch (err) {
    log('snapshot write failed: %o', err);
  }
}

/**
 * Fetch the mascot manifest, memoised for the session. On a network failure we
 * fall back to the last localStorage snapshot so the picker still works
 * offline; if there is no snapshot either, the rejection propagates so the UI
 * can show an error state.
 */
export function fetchMascotManifest(): Promise<MascotManifest> {
  if (inflight) return inflight;
  inflight = (async () => {
    const controller = new AbortController();
    const timeoutId = window.setTimeout(() => controller.abort(), MANIFEST_FETCH_TIMEOUT_MS);
    try {
      log('fetching manifest %s', MASCOT_MANIFEST_URL);
      const res = await fetch(MASCOT_MANIFEST_URL, {
        cache: 'no-cache',
        signal: controller.signal,
      });
      if (!res.ok) throw new Error(`manifest fetch failed (${res.status})`);
      const manifest = parseManifest(await res.json());
      log('manifest ok — %d mascots (schema v%d)', manifest.mascots.length, manifest.schemaVersion);
      writeSnapshot(manifest);
      return manifest;
    } catch (err) {
      const snapshot = readSnapshot();
      if (snapshot) {
        log('manifest fetch failed, using snapshot: %o', err);
        return snapshot;
      }
      // Reset so a later retry can attempt the network again rather than
      // re-resolving this rejected promise forever.
      inflight = null;
      throw err;
    } finally {
      window.clearTimeout(timeoutId);
    }
  })();
  return inflight;
}

/** Drop the memoised fetch so the next call re-hits the network. */
export function clearManifestCache(): void {
  inflight = null;
}

/** Find a mascot entry by id within a manifest. */
export function findMascot(
  manifest: MascotManifest,
  id: string | null | undefined
): MascotManifestEntry | undefined {
  if (!id) return undefined;
  return manifest.mascots.find(m => m.id === id);
}

/** The mascot the app defaults to: the first `ready` entry, else the first. */
export function defaultMascot(manifest: MascotManifest): MascotManifestEntry | undefined {
  return manifest.mascots.find(m => m.status === 'ready') ?? manifest.mascots[0];
}

/**
 * Resolve a mascot's `.riv` binary, version-cached in IndexedDB by its
 * runtime file's sha256. The network is only hit when the sha changes (i.e.
 * the asset was republished), so re-selecting a mascot is instant.
 */
export async function loadManifestRiv(entry: MascotManifestEntry): Promise<ArrayBuffer> {
  const runtime = runtimeFile(entry);
  if (!runtime) throw new Error(`mascot ${entry.id} has no runtime .riv file`);
  return loadRivBuffer(entry.id, runtime.sha256, runtime.url);
}
