/**
 * Types for the GitHub-published mascot manifest
 * (`tinyhumansai/mascots` → `dist/mascots.json`).
 *
 * This is the authoritative source for the in-app mascot library: each entry
 * names a Rive runtime file plus a `stateEngine` describing the poses, logical
 * states, viseme vocabulary, and optional secondary animation channels (e.g.
 * darting eyes) the asset's `MascotSM` state machine accepts.
 *
 * Schema mirror: `tinyhumansai/mascots:schemas/mascots.schema.json` (v1). Kept
 * as hand-written TS rather than generated because the app fetches the manifest
 * directly over HTTPS — there is no shared package between the two repos.
 */

/** One downloadable asset belonging to a mascot. */
export interface MascotManifestFile {
  /** Repo-relative path, e.g. `mascots/tiny-mascot/tinyMascot.riv`. */
  path: string;
  bytes: number;
  /** `runtime` = the playable `.riv`; `source` = the `.rev` editor file. */
  role: 'runtime' | 'source';
  /** Lowercase hex sha256 of the file. Doubles as the cache-bust version key. */
  sha256: string;
  /** Absolute `raw.githubusercontent.com` URL the app fetches. */
  url: string;
}

/** Optional secondary animation channel (its own enum input on the view model). */
export interface MascotManifestChannel {
  /** View-model enum input name to drive (e.g. `eyes`). */
  key: string;
  /** Enum name inside the `.riv` (informational). */
  enum?: string;
  /** Human label for the controls UI. */
  label?: string;
  /** Every value the enum accepts. */
  values: string[];
  /** Default value; falls back to `values[0]` when absent. */
  default?: string;
  /** Auto-cycle config — drives idle "aliveness" (e.g. random eye darts). */
  cycle?: { enabled: boolean; intervalMs?: number; order?: 'random' | 'sequential' };
}

/** The animation contract a mascot's `.riv` is authored against. */
export interface MascotStateEngine {
  /**
   * Ordered viseme codes the asset's mouth enum accepts. Standard
   * Oculus/ElevenLabs 15-set: `sil, PP, FF, TH, DD, kk, CH, SS, nn, RR, aa, E,
   * ih, oh, ou`. The first entry (`sil`) is the resting/closed mouth.
   */
  visemeCodes: string[];
  /** Logical state → pose-enum value. Always carries at least `idle`/`thinking`. */
  states: { idle: string; thinking: string; [key: string]: string };
  /** Poses the mascot drifts through while idle (includes the resting pose). */
  idlePoseCycle: string[];
  /** Optional extra enum channels (eyes, etc.). */
  channels?: MascotManifestChannel[];
}

/** A single mascot in the manifest. */
export interface MascotManifestEntry {
  id: string;
  name: string;
  description: string;
  /** `ready` = production; `draft` = not yet matched to the runtime contract. */
  status: 'ready' | 'draft';
  tags: string[];
  stateEngine: MascotStateEngine;
  files: MascotManifestFile[];
}

/** Top-level manifest document. */
export interface MascotManifest {
  schemaVersion: number;
  generatedAt: string;
  mascots: MascotManifestEntry[];
  source: { repository: string; branch: string; commit: string };
}

/** The runtime (`.riv`) file for a mascot, or `undefined` if it has none. */
export function runtimeFile(entry: MascotManifestEntry): MascotManifestFile | undefined {
  return entry.files.find(f => f.role === 'runtime');
}

/** The source (`.rev`) file for a mascot, or `undefined` if it has none. */
export function sourceFile(entry: MascotManifestEntry): MascotManifestFile | undefined {
  return entry.files.find(f => f.role === 'source');
}
