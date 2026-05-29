import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('skillsApi');

/**
 * Scope a skill was discovered in.
 *
 * Mirrors `openhuman::skills::ops::SkillScope` on the Rust side — serialized
 * as a lowercase string (`"user" | "project" | "legacy"`).
 */
export type SkillScope = 'user' | 'project' | 'legacy';

/**
 * Wire-format representation of a discovered skill returned by
 * `openhuman.skills_list`.
 *
 * Paths are intentionally serialized as strings (not URLs) to avoid lossy
 * conversions on non-UTF-8 filesystems.
 */
export interface SkillSummary {
  /** Stable identifier — equal to `name` on the Rust side. */
  id: string;
  /** Display name, from frontmatter or directory. */
  name: string;
  /** Short prose summary from frontmatter / `description`. */
  description: string;
  /** Version string, if declared (empty otherwise). */
  version: string;
  /** Author string, if declared. */
  author: string | null;
  /** Tags declared in frontmatter metadata. */
  tags: string[];
  /** Tool hint from `allowed-tools`. */
  tools: string[];
  /** Prompt files declared in the legacy manifest. */
  prompts: string[];
  /** Path to `SKILL.md` (or `skill.json`) on disk, or null if unknown. */
  location: string | null;
  /** Bundled resource files, relative to the skill root. */
  resources: string[];
  /** Where the skill came from. */
  scope: SkillScope;
  /** True when loaded from the legacy `skills/` layout. */
  legacy: boolean;
  /** Non-fatal parse warnings to surface in the UI. */
  warnings: string[];
}

interface SkillsListResult {
  skills: SkillSummary[];
}

/**
 * Result of `openhuman.skills_read_resource`.
 */
export interface SkillResourceContent {
  /** Echo of the requested skill id. */
  skillId: string;
  /** Echo of the requested relative path. */
  relativePath: string;
  /** UTF-8 file contents (<= 128 KB). */
  content: string;
  /** Size of the file on disk, in bytes. */
  bytes: number;
}

interface RawSkillsReadResourceResult {
  skill_id: string;
  relative_path: string;
  content: string;
  bytes: number;
}

/**
 * Parameters accepted by `openhuman.skills_create`.
 *
 * Matches the wire shape defined in `src/openhuman/skills/schemas.rs`
 * (`SkillsCreateParams`) — `allowedTools` is rekeyed to `allowed-tools` on
 * the JSON-RPC envelope per SKILL.md frontmatter convention (with
 * `allowed_tools` accepted as an alias by the Rust deserializer).
 */
/**
 * One declared `[[inputs]]` row supplied at create time by
 * `CreateSkillForm.tsx`. Mirrors the Rust `SkillCreateInputDef` wire
 * shape — `description` and `type` are optional; `required` defaults
 * to `true` on the Rust side when omitted (we send it explicitly to
 * stay loud).
 */
export interface CreateSkillInputDef {
  name: string;
  description?: string;
  required: boolean;
  type?: 'string' | 'integer' | 'boolean';
}

export interface CreateSkillInput {
  name: string;
  description: string;
  scope?: SkillScope;
  license?: string;
  author?: string;
  tags?: string[];
  allowedTools?: string[];
  /**
   * Optional list of `[[inputs]]` rows. When non-empty the Rust side
   * writes a sibling `skill.toml` next to the generated SKILL.md so
   * the Skills Runner can render dynamic form controls per input.
   * Omit / pass `[]` to scaffold an input-less skill.
   */
  inputs?: CreateSkillInputDef[];
}

interface RawSkillsCreateResult {
  skill: SkillSummary;
}

/**
 * Parameters accepted by `openhuman.skills_install_from_url`.
 *
 * `timeoutSecs` is optional — the Rust side defaults to 60s and caps at
 * 600s. Values outside that range are clamped server-side.
 */
export interface InstallSkillFromUrlInput {
  url: string;
  timeoutSecs?: number;
}

/**
 * Result of `openhuman.skills_install_from_url`.
 *
 * `newSkills` lists skill ids that appeared post-install (diff vs the
 * pre-install snapshot). `stdout` holds a human-readable diagnostic summary
 * (bytes fetched, target path); `stderr` holds non-fatal frontmatter parse
 * warnings joined by newlines. There is no subprocess — the Rust side fetches
 * SKILL.md directly over HTTPS.
 */
export interface InstallSkillFromUrlResult {
  url: string;
  stdout: string;
  stderr: string;
  newSkills: string[];
}

interface RawInstallSkillFromUrlResult {
  url: string;
  stdout: string;
  stderr: string;
  new_skills: string[];
}

/**
 * Result of `openhuman.skills_uninstall`.
 *
 * Mirrors the Rust-side `UninstallSkillOutcome`. `removedPath` is the
 * canonicalised on-disk path that was deleted — surface it in success toasts
 * so the user can confirm exactly what was removed.
 */
export interface UninstallSkillResult {
  name: string;
  removedPath: string;
  scope: SkillScope;
}

interface RawUninstallSkillResult {
  name: string;
  removed_path: string;
  scope: SkillScope;
}

interface Envelope<T> {
  data?: T;
}

function unwrapEnvelope<T>(response: Envelope<T> | T): T {
  if (response && typeof response === 'object' && 'data' in response) {
    const envelope = response as Envelope<T>;
    if (envelope.data !== undefined) {
      return envelope.data as T;
    }
  }
  return response as T;
}

export const skillsApi = {
  /** Enumerate SKILL.md / legacy skills visible in the active workspace. */
  listSkills: async (): Promise<SkillSummary[]> => {
    log('listSkills: request');
    const response = await callCoreRpc<Envelope<SkillsListResult> | SkillsListResult>({
      method: 'openhuman.skills_list',
    });
    const result = unwrapEnvelope(response);
    const skills = result?.skills ?? [];
    log('listSkills: response count=%d', skills.length);
    return skills;
  },

  /**
   * Read a single bundled resource file from a discovered skill. Rejects on
   * traversal, symlink escape, non-UTF-8 payloads, or files larger than
   * 128 KB — the caller surfaces the error string verbatim in the drawer.
   */
  readSkillResource: async ({
    skillId,
    relativePath,
  }: {
    skillId: string;
    relativePath: string;
  }): Promise<SkillResourceContent> => {
    log('readSkillResource: request skillId=%s path=%s', skillId, relativePath);
    const response = await callCoreRpc<
      Envelope<RawSkillsReadResourceResult> | RawSkillsReadResourceResult
    >({
      method: 'openhuman.skills_read_resource',
      params: { skill_id: skillId, relative_path: relativePath },
    });
    const raw = unwrapEnvelope(response);
    const normalized: SkillResourceContent = {
      skillId: raw.skill_id,
      relativePath: raw.relative_path,
      content: raw.content,
      bytes: raw.bytes,
    };
    log('readSkillResource: response bytes=%d', normalized.bytes);
    return normalized;
  },

  /**
   * Scaffold a new SKILL.md skill via `openhuman.skills_create`.
   *
   * The Rust side slugifies the name, writes `SKILL.md` with the supplied
   * frontmatter, and returns the freshly-discovered `SkillSummary` so the
   * caller can insert the new row into the grid without a full refetch.
   */
  createSkill: async (input: CreateSkillInput): Promise<SkillSummary> => {
    log('createSkill: request name=%s scope=%s', input.name, input.scope ?? 'default');
    const response = await callCoreRpc<Envelope<RawSkillsCreateResult> | RawSkillsCreateResult>({
      method: 'openhuman.skills_create',
      params: {
        name: input.name,
        description: input.description,
        ...(input.scope !== undefined ? { scope: input.scope } : {}),
        ...(input.license !== undefined ? { license: input.license } : {}),
        ...(input.author !== undefined ? { author: input.author } : {}),
        ...(input.tags !== undefined ? { tags: input.tags } : {}),
        ...(input.allowedTools !== undefined ? { 'allowed-tools': input.allowedTools } : {}),
        ...(input.inputs !== undefined && input.inputs.length > 0 ? { inputs: input.inputs } : {}),
      },
    });
    const raw = unwrapEnvelope(response);
    log('createSkill: response id=%s', raw.skill.id);
    return raw.skill;
  },

  /**
   * Install a remote SKILL.md by URL via `openhuman.skills_install_from_url`.
   *
   * The Rust side fetches the SKILL.md directly over HTTPS (no subprocess,
   * no Node toolchain required), validates the frontmatter, and writes it
   * into the user-scope skills directory. URL must be https, resolve to a
   * public host, and point at a single `.md` file; `github.com/.../blob/...`
   * is normalised to its `raw.githubusercontent.com` equivalent. Size is
   * capped at 1 MiB; timeout default 60s, max 600s.
   */
  installSkillFromUrl: async (
    input: InstallSkillFromUrlInput
  ): Promise<InstallSkillFromUrlResult> => {
    log('installSkillFromUrl: request url=%s', input.url);
    const response = await callCoreRpc<
      Envelope<RawInstallSkillFromUrlResult> | RawInstallSkillFromUrlResult
    >({
      method: 'openhuman.skills_install_from_url',
      params: {
        url: input.url,
        ...(input.timeoutSecs !== undefined ? { timeout_secs: input.timeoutSecs } : {}),
      },
    });
    const raw = unwrapEnvelope(response);
    const normalized: InstallSkillFromUrlResult = {
      url: raw.url,
      stdout: raw.stdout,
      stderr: raw.stderr,
      newSkills: raw.new_skills ?? [],
    };
    log(
      'installSkillFromUrl: response new=%d stdout=%d stderr=%d',
      normalized.newSkills.length,
      normalized.stdout.length,
      normalized.stderr.length
    );
    return normalized;
  },

  /**
   * Remove an installed user-scope SKILL.md skill via `openhuman.skills_uninstall`.
   *
   * Only user-scope installs (`~/.openhuman/skills/<name>/`) are supported.
   * Project-scope and legacy skills are read-only — trying to uninstall one
   * returns a backend error surfaced as a rejected promise. The Rust side
   * canonicalises paths and refuses names with separators / traversal
   * sequences / anything outside the skills root.
   */
  uninstallSkill: async (name: string): Promise<UninstallSkillResult> => {
    log('uninstallSkill: request name=%s', name);
    const response = await callCoreRpc<Envelope<RawUninstallSkillResult> | RawUninstallSkillResult>(
      { method: 'openhuman.skills_uninstall', params: { name } }
    );
    const raw = unwrapEnvelope(response);
    const normalized: UninstallSkillResult = {
      name: raw.name,
      removedPath: raw.removed_path,
      scope: raw.scope,
    };
    log('uninstallSkill: response name=%s removedPath=%s', normalized.name, normalized.removedPath);
    return normalized;
  },

  /**
   * Fetch the declared `[[inputs]]` for a single skill plus its display
   * metadata. Lightweight companion to `listSkills` — `SkillSummary` rows
   * (used by the catalog grid) deliberately don't include input
   * declarations, so the Skills Runner panel calls this once when the
   * user picks a skill from the dropdown so it can render the right form
   * controls.
   */
  describeSkill: async (skillId: string): Promise<SkillDescription> => {
    log('describeSkill: request skillId=%s', skillId);
    const response = await callCoreRpc<Envelope<SkillDescription> | SkillDescription>({
      method: 'openhuman.skills_describe',
      params: { skill_id: skillId },
    });
    const raw = unwrapEnvelope(response);
    log('describeSkill: response inputs=%d', raw.inputs.length);
    return raw;
  },

  /**
   * Fire-and-forget invocation of `openhuman.skills_run`. Returns
   * immediately with the new background run's `run_id`, the canonical
   * `skill_id`, and the log path the run is streaming into; the actual
   * autonomous work continues in the background and finishes with
   * status `DONE` / `DEGENERATE` / `FAILED` in the run log.
   */
  runSkill: async (skillId: string, inputs: Record<string, unknown>): Promise<SkillRunStarted> => {
    log('runSkill: request skillId=%s', skillId);
    const response = await callCoreRpc<Envelope<SkillRunStarted> | SkillRunStarted>({
      method: 'openhuman.skills_run',
      params: { skill_id: skillId, inputs },
    });
    const raw = unwrapEnvelope(response);
    log('runSkill: response runId=%s log=%s', raw.run_id, raw.log);
    return raw;
  },

  /**
   * Read a slice of a skill run's streaming log file by run_id. Pass
   * `offset` to tail forward — the returned `offset` is the cursor for
   * the next call. Stop polling once `complete: true` (footer landed).
   */
  readRunLog: async (runId: string, offset?: number, maxBytes?: number): Promise<RunLogSlice> => {
    log(
      'readRunLog: request runId=%s offset=%s maxBytes=%s',
      runId,
      offset ?? 0,
      maxBytes ?? 'default'
    );
    const params: Record<string, unknown> = { run_id: runId };
    if (offset !== undefined) params.offset = offset;
    if (maxBytes !== undefined) params.max_bytes = maxBytes;
    const response = await callCoreRpc<Envelope<RunLogSlice> | RunLogSlice>({
      method: 'openhuman.skills_read_run_log',
      params,
    });
    const raw = unwrapEnvelope(response);
    log('readRunLog: response bytes=%d eof=%s complete=%s', raw.bytes_read, raw.eof, raw.complete);
    return raw;
  },

  /**
   * Recent autonomous skill runs from `<workspace>/skills/.runs/`. Sorted
   * by start time descending. Pass `skillId` to filter to one skill,
   * omit for cross-skill. `limit` defaults to 20 (max 100).
   */
  recentRuns: async (skillId?: string, limit?: number): Promise<ScannedRun[]> => {
    log('recentRuns: request skillId=%s limit=%s', skillId ?? '*', limit ?? 'default');
    const params: Record<string, unknown> = {};
    if (skillId !== undefined) params.skill_id = skillId;
    if (limit !== undefined) params.limit = limit;
    const response = await callCoreRpc<Envelope<{ runs: ScannedRun[] }> | { runs: ScannedRun[] }>({
      method: 'openhuman.skills_recent_runs',
      params,
    });
    const raw = unwrapEnvelope(response);
    log('recentRuns: response count=%d', raw.runs.length);
    return raw.runs;
  },
};

/**
 * One input declaration from a skill's `[[inputs]]` block, returned by
 * `openhuman.skills_describe`. The FE renders one form control per entry:
 * `string`/`integer`/`boolean` map to text/number/checkbox controls.
 */
export interface SkillInputDescription {
  name: string;
  description: string;
  required: boolean;
  /** Type hint from `[[inputs]].type`. */
  type: string;
}

/** Wire shape returned by `openhuman.skills_describe`. */
export interface SkillDescription {
  id: string;
  display_name: string;
  when_to_use: string;
  inputs: SkillInputDescription[];
}

/** Wire shape returned by `openhuman.skills_run` (fire-and-forget). */
export interface SkillRunStarted {
  run_id: string;
  status: string; // "started"
  skill_id: string;
  log: string; // absolute path to the streaming log
}

/**
 * Slice of a run log file returned by `openhuman.skills_read_run_log`.
 * Mirrors `crate::openhuman::skills::run_log::RunLogSlice`. The FE
 * passes the returned `offset` as the next call's `offset` to tail
 * forward; polling can stop once `complete: true` (the `--- result ---`
 * footer has landed in the file).
 */
export interface RunLogSlice {
  /** New read cursor — next call's `offset`. */
  offset: number;
  bytes_read: number;
  content: string;
  /** True if the read reached end-of-file (may still be incomplete). */
  eof: boolean;
  /** True once the run footer landed in the file. FE stops polling. */
  complete: boolean;
}

/**
 * One run entry returned by `openhuman.skills_recent_runs`. Wire shape
 * mirrors `crate::openhuman::skills::run_log::ScannedRun`. `status` is
 * `"RUNNING"` while the run hasn't written its `--- result ---` footer
 * yet; after the footer lands it becomes `"DONE"` / `"DEGENERATE"` /
 * `"FAILED"`.
 */
export interface ScannedRun {
  run_id: string;
  skill_id: string;
  /** RFC3339-with-trailing-`UTC` timestamp from the log header. */
  started: string;
  status: 'RUNNING' | 'DONE' | 'DEGENERATE' | 'FAILED' | string;
  /** Footer `duration: <ms> ms`. Null while running. */
  duration_ms: number | null;
  /** Footer `finished:` timestamp. Null while running. */
  finished: string | null;
  /** Absolute path to the streaming log file. */
  log_path: string;
}
