import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('workflowsApi');

/**
 * Scope a skill was discovered in.
 *
 * Mirrors `openhuman::skills::ops::WorkflowScope` on the Rust side — serialized
 * as a lowercase string (`"user" | "project" | "legacy"`).
 */
export type WorkflowScope = 'user' | 'project' | 'legacy';

/**
 * Wire-format representation of a discovered skill returned by
 * `openhuman.workflows_list`.
 *
 * Paths are intentionally serialized as strings (not URLs) to avoid lossy
 * conversions on non-UTF-8 filesystems.
 */
export interface WorkflowSummary {
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
  /** Platform compatibility hints from SKILL.md frontmatter. */
  platforms: string[];
  /** Related skills declared by the originating ecosystem. */
  relatedSkills: string[];
  /** Normalized source format hint, e.g. openhuman, hermes, legacy. */
  sourceFormat: string;
  /** Tool hint from `allowed-tools`. */
  tools: string[];
  /** Prompt files declared in the legacy manifest. */
  prompts: string[];
  /** Path to `SKILL.md` (or `skill.json`) on disk, or null if unknown. */
  location: string | null;
  /** Bundled resource files, relative to the skill root. */
  resources: string[];
  /** Where the skill came from. */
  scope: WorkflowScope;
  /** True when loaded from the legacy `skills/` layout. */
  legacy: boolean;
  /** Non-fatal parse warnings to surface in the UI. */
  warnings: string[];
}

interface WorkflowsListResult {
  workflows: RawWorkflowSummary[];
}

type RawWorkflowSummary = Omit<WorkflowSummary, 'platforms' | 'relatedSkills' | 'sourceFormat'> & {
  platforms?: string[];
  related_skills?: string[];
  relatedSkills?: string[];
  source_format?: string;
  sourceFormat?: string;
};

/**
 * Result of `openhuman.workflows_read_resource`.
 */
export interface WorkflowResourceContent {
  /** Echo of the requested skill id. */
  workflowId: string;
  /** Echo of the requested relative path. */
  relativePath: string;
  /** UTF-8 file contents (<= 128 KB). */
  content: string;
  /** Size of the file on disk, in bytes. */
  bytes: number;
}

interface RawWorkflowsReadResourceResult {
  workflow_id: string;
  relative_path: string;
  content: string;
  bytes: number;
}

/**
 * Parameters accepted by `openhuman.workflows_create`.
 *
 * Matches the wire shape defined in `src/openhuman/skills/schemas.rs`
 * (`SkillsCreateParams`) — `allowedTools` is rekeyed to `allowed-tools` on
 * the JSON-RPC envelope per SKILL.md frontmatter convention (with
 * `allowed_tools` accepted as an alias by the Rust deserializer).
 */
/**
 * One declared `[[inputs]]` row supplied at create time by
 * `CreateWorkflowForm.tsx`. Mirrors the Rust `SkillCreateInputDef` wire
 * shape — `description` and `type` are optional; `required` defaults
 * to `true` on the Rust side when omitted (we send it explicitly to
 * stay loud).
 */
export interface CreateWorkflowInputDef {
  name: string;
  description?: string;
  required: boolean;
  type?: 'string' | 'integer' | 'boolean';
}

export interface CreateWorkflowInput {
  name: string;
  description: string;
  /**
   * Optional trigger/goal — *when* an agent should reach for this workflow.
   * This is the workflow half of the unified form (a bare procedure md only
   * says what it does, not when to run it). Persisted to the workflow's
   * `skill.toml` `when_to_use`; falls back to `description` when omitted.
   */
  whenToUse?: string;
  scope?: WorkflowScope;
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
  inputs?: CreateWorkflowInputDef[];
}

interface RawWorkflowsCreateResult {
  workflow: RawWorkflowSummary;
}

/**
 * Parameters accepted by `openhuman.workflows_install_from_url`.
 *
 * `timeoutSecs` is optional — the Rust side defaults to 60s and caps at
 * 600s. Values outside that range are clamped server-side.
 */
export interface InstallWorkflowFromUrlInput {
  url: string;
  timeoutSecs?: number;
}

/**
 * Result of `openhuman.workflows_install_from_url`.
 *
 * `newWorkflows` lists skill ids that appeared post-install (diff vs the
 * pre-install snapshot). `stdout` holds a human-readable diagnostic summary
 * (bytes fetched, target path); `stderr` holds non-fatal frontmatter parse
 * warnings joined by newlines. There is no subprocess — the Rust side fetches
 * SKILL.md directly over HTTPS.
 */
export interface InstallWorkflowFromUrlResult {
  url: string;
  stdout: string;
  stderr: string;
  newWorkflows: string[];
}

interface RawInstallWorkflowFromUrlResult {
  url: string;
  stdout: string;
  stderr: string;
  new_workflows: string[];
}

/**
 * Result of `openhuman.skill_registry_uninstall`.
 *
 * Mirrors the Rust-side `UninstallSkillOutcome`. `removedPath` is the
 * canonicalised on-disk path that was deleted — surface it in success toasts
 * so the user can confirm exactly what was removed.
 */
export interface UninstallWorkflowResult {
  name: string;
  removedPath: string;
  scope: WorkflowScope;
}

interface RawUninstallWorkflowResult {
  name: string;
  removed_path: string;
  scope: WorkflowScope;
}

export interface SkillRuntimeSummary {
  runtime: 'node' | 'python' | string;
  enabled: boolean;
  available: boolean;
  source: 'system' | 'managed' | string | null;
  version: string | null;
  binary: string | null;
  binDir: string | null;
  error: string | null;
}

interface RawSkillRuntimeSummary {
  runtime: string;
  enabled: boolean;
  available: boolean;
  source: string | null;
  version: string | null;
  binary: string | null;
  bin_dir: string | null;
  error: string | null;
}

export interface ResolveSkillRuntimesResult {
  runtimes: SkillRuntimeSummary[];
}

interface RawResolveSkillRuntimesResult {
  runtimes: RawSkillRuntimeSummary[];
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

function normalizeWorkflowSummary(raw: RawWorkflowSummary): WorkflowSummary {
  return {
    ...raw,
    platforms: raw.platforms ?? [],
    relatedSkills: raw.relatedSkills ?? raw.related_skills ?? [],
    sourceFormat: raw.sourceFormat ?? raw.source_format ?? (raw.legacy ? 'legacy' : 'openhuman'),
  };
}

/** Options for {@link workflowsApi.listWorkflows}. */
export interface ListWorkflowsOptions {
  /**
   * When `true`, also include capability skills under the `skills/` roots
   * (registry installs land there), not just `workflows/`-root automations.
   */
  includeSkills?: boolean;
}

export const workflowsApi = {
  /**
   * Enumerate SKILL.md / legacy skills visible in the active workspace.
   *
   * By default returns only `workflows/`-root automations (the Automations UI
   * view). Pass `{ includeSkills: true }` to also include capability skills
   * under the `skills/` roots — the Skills Explorer uses this so
   * registry-installed skills show up in its Installed tab.
   */
  listWorkflows: async (opts?: ListWorkflowsOptions): Promise<WorkflowSummary[]> => {
    log('listWorkflows: request includeSkills=%s', opts?.includeSkills ?? false);
    const response = await callCoreRpc<Envelope<WorkflowsListResult> | WorkflowsListResult>({
      method: 'openhuman.workflows_list',
      params: opts?.includeSkills ? { include_skills: true } : undefined,
    });
    const result = unwrapEnvelope(response);
    const workflows = (result?.workflows ?? []).map(normalizeWorkflowSummary);
    log('listWorkflows: response count=%d', workflows.length);
    return workflows;
  },

  /**
   * Read a single bundled resource file from a discovered skill. Rejects on
   * traversal, symlink escape, non-UTF-8 payloads, or files larger than
   * 128 KB — the caller surfaces the error string verbatim in the drawer.
   */
  readWorkflowResource: async ({
    workflowId,
    relativePath,
  }: {
    workflowId: string;
    relativePath: string;
  }): Promise<WorkflowResourceContent> => {
    log('readWorkflowResource: request workflowId=%s path=%s', workflowId, relativePath);
    const response = await callCoreRpc<
      Envelope<RawWorkflowsReadResourceResult> | RawWorkflowsReadResourceResult
    >({
      method: 'openhuman.workflows_read_resource',
      params: { workflow_id: workflowId, relative_path: relativePath },
    });
    const raw = unwrapEnvelope(response);
    const normalized: WorkflowResourceContent = {
      workflowId: raw.workflow_id,
      relativePath: raw.relative_path,
      content: raw.content,
      bytes: raw.bytes,
    };
    log('readWorkflowResource: response bytes=%d', normalized.bytes);
    return normalized;
  },

  /**
   * Scaffold a new SKILL.md skill via `openhuman.workflows_create`.
   *
   * The Rust side slugifies the name, writes `SKILL.md` with the supplied
   * frontmatter, and returns the freshly-discovered `WorkflowSummary` so the
   * caller can insert the new row into the grid without a full refetch.
   */
  createWorkflow: async (input: CreateWorkflowInput): Promise<WorkflowSummary> => {
    log('createWorkflow: request name=%s scope=%s', input.name, input.scope ?? 'default');
    const response = await callCoreRpc<
      Envelope<RawWorkflowsCreateResult> | RawWorkflowsCreateResult
    >({
      method: 'openhuman.workflows_create',
      params: {
        name: input.name,
        description: input.description,
        ...(input.whenToUse !== undefined && input.whenToUse.trim().length > 0
          ? { when_to_use: input.whenToUse }
          : {}),
        ...(input.scope !== undefined ? { scope: input.scope } : {}),
        ...(input.license !== undefined ? { license: input.license } : {}),
        ...(input.author !== undefined ? { author: input.author } : {}),
        ...(input.tags !== undefined ? { tags: input.tags } : {}),
        ...(input.allowedTools !== undefined ? { 'allowed-tools': input.allowedTools } : {}),
        ...(input.inputs !== undefined && input.inputs.length > 0 ? { inputs: input.inputs } : {}),
      },
    });
    const raw = unwrapEnvelope(response);
    const workflow = normalizeWorkflowSummary(raw.workflow);
    log('createWorkflow: response id=%s', workflow.id);
    return workflow;
  },

  /**
   * Edit an existing workflow via `openhuman.workflows_update`. Same payload
   * shape as create; the Rust side overwrites the workflow at the resolved
   * slug — rewriting frontmatter + workflow.toml while preserving the
   * hand-authored SKILL.md/WORKFLOW.md body.
   */
  updateWorkflow: async (input: CreateWorkflowInput): Promise<WorkflowSummary> => {
    log('updateWorkflow: request name=%s scope=%s', input.name, input.scope ?? 'default');
    const response = await callCoreRpc<
      Envelope<RawWorkflowsCreateResult> | RawWorkflowsCreateResult
    >({
      method: 'openhuman.workflows_update',
      params: {
        name: input.name,
        description: input.description,
        ...(input.whenToUse !== undefined && input.whenToUse.trim().length > 0
          ? { when_to_use: input.whenToUse }
          : {}),
        ...(input.scope !== undefined ? { scope: input.scope } : {}),
        ...(input.license !== undefined ? { license: input.license } : {}),
        ...(input.author !== undefined ? { author: input.author } : {}),
        ...(input.tags !== undefined ? { tags: input.tags } : {}),
        ...(input.allowedTools !== undefined ? { 'allowed-tools': input.allowedTools } : {}),
        ...(input.inputs !== undefined && input.inputs.length > 0 ? { inputs: input.inputs } : {}),
      },
    });
    const raw = unwrapEnvelope(response);
    const workflow = normalizeWorkflowSummary(raw.workflow);
    log('updateWorkflow: response id=%s', workflow.id);
    return workflow;
  },

  /**
   * Install a remote SKILL.md by URL via `openhuman.workflows_install_from_url`.
   *
   * The Rust side fetches the SKILL.md directly over HTTPS (no subprocess,
   * no Node toolchain required), validates the frontmatter, and writes it
   * into the user-scope skills directory. URL must be https, resolve to a
   * public host, and point at a single `.md` file; `github.com/.../blob/...`
   * is normalised to its `raw.githubusercontent.com` equivalent. Size is
   * capped at 1 MiB; timeout default 60s, max 600s.
   */
  installWorkflowFromUrl: async (
    input: InstallWorkflowFromUrlInput
  ): Promise<InstallWorkflowFromUrlResult> => {
    log('installWorkflowFromUrl: request url=%s', input.url);
    const response = await callCoreRpc<
      Envelope<RawInstallWorkflowFromUrlResult> | RawInstallWorkflowFromUrlResult
    >({
      method: 'openhuman.workflows_install_from_url',
      params: {
        url: input.url,
        ...(input.timeoutSecs !== undefined ? { timeout_secs: input.timeoutSecs } : {}),
      },
    });
    const raw = unwrapEnvelope(response);
    const normalized: InstallWorkflowFromUrlResult = {
      url: raw.url,
      stdout: raw.stdout,
      stderr: raw.stderr,
      newWorkflows: raw.new_workflows ?? [],
    };
    log(
      'installWorkflowFromUrl: response new=%d stdout=%d stderr=%d',
      normalized.newWorkflows.length,
      normalized.stdout.length,
      normalized.stderr.length
    );
    return normalized;
  },

  /**
   * Remove an installed user-scope SKILL.md skill via `openhuman.skill_registry_uninstall`.
   *
   * Only user-scope installs (`~/.openhuman/skills/<name>/`) are supported.
   * Project-scope and legacy skills are read-only — trying to uninstall one
   * returns a backend error surfaced as a rejected promise. The Rust side
   * canonicalises paths and refuses names with separators / traversal
   * sequences / anything outside the skills root.
   */
  uninstallWorkflow: async (name: string): Promise<UninstallWorkflowResult> => {
    log('uninstallWorkflow: request name=%s', name);
    const response = await callCoreRpc<
      Envelope<RawUninstallWorkflowResult> | RawUninstallWorkflowResult
    >({ method: 'openhuman.skill_registry_uninstall', params: { name } });
    const raw = unwrapEnvelope(response);
    const normalized: UninstallWorkflowResult = {
      name: raw.name,
      removedPath: raw.removed_path,
      scope: raw.scope,
    };
    log(
      'uninstallWorkflow: response name=%s removedPath=%s',
      normalized.name,
      normalized.removedPath
    );
    return normalized;
  },

  /**
   * Fetch the declared `[[inputs]]` for a single skill plus its display
   * metadata. Lightweight companion to `listWorkflows` — `WorkflowSummary` rows
   * (used by the catalog grid) deliberately don't include input
   * declarations, so the Skills Runner panel calls this once when the
   * user picks a skill from the dropdown so it can render the right form
   * controls.
   */
  describeWorkflow: async (workflowId: string): Promise<WorkflowDescription> => {
    log('describeWorkflow: request workflowId=%s', workflowId);
    const response = await callCoreRpc<Envelope<WorkflowDescription> | WorkflowDescription>({
      method: 'openhuman.workflows_describe',
      params: { workflow_id: workflowId },
    });
    const raw = unwrapEnvelope(response);
    log('describeWorkflow: response inputs=%d', raw.inputs.length);
    return raw;
  },

  /**
   * Fire-and-forget invocation of `openhuman.skill_runtime_run`. Returns
   * immediately with the new background run's `run_id`, the canonical
   * skill/workflow id, and the log path the run is streaming into; the actual
   * autonomous work continues in the background and finishes with
   * status `DONE` / `DEGENERATE` / `FAILED` in the run log.
   */
  runWorkflow: async (
    workflowId: string,
    inputs: Record<string, unknown>
  ): Promise<WorkflowRunStarted> => {
    log('runWorkflow: request workflowId=%s', workflowId);
    const response = await callCoreRpc<Envelope<RawSkillRunStarted> | RawSkillRunStarted>({
      method: 'openhuman.skill_runtime_run',
      params: { skill_id: workflowId, inputs },
    });
    const raw = unwrapEnvelope(response);
    const normalized: WorkflowRunStarted = {
      run_id: raw.run_id,
      status: raw.status,
      workflow_id: raw.workflow_id ?? raw.skill_id,
      log: raw.log,
    };
    log('runWorkflow: response runId=%s log=%s', normalized.run_id, normalized.log);
    return normalized;
  },
  /**
   * Request cancellation of an in-flight run via `openhuman.skill_runtime_cancel`.
   * Returns `true` if a live run with this id was found and signalled; the run
   * stops at its next await and lands a CANCELLED footer.
   */
  cancelRun: async (runId: string): Promise<boolean> => {
    log('cancelRun: request runId=%s', runId);
    const response = await callCoreRpc<Envelope<{ cancelled: boolean }> | { cancelled: boolean }>({
      method: 'openhuman.skill_runtime_cancel',
      params: { run_id: runId },
    });
    const raw = unwrapEnvelope(response);
    log('cancelRun: response cancelled=%s', raw.cancelled);
    return raw.cancelled;
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
      method: 'openhuman.skill_runtime_read_run_log',
      params,
    });
    const raw = unwrapEnvelope(response);
    log('readRunLog: response bytes=%d eof=%s complete=%s', raw.bytes_read, raw.eof, raw.complete);
    return raw;
  },

  /**
   * Recent autonomous skill runs from `<workspace>/skills/.runs/`. Sorted
   * by start time descending. Pass `workflowId` to filter to one skill,
   * omit for cross-skill. `limit` defaults to 20 (max 100).
   */
  recentRuns: async (workflowId?: string, limit?: number): Promise<ScannedRun[]> => {
    log('recentRuns: request workflowId=%s limit=%s', workflowId ?? '*', limit ?? 'default');
    const params: Record<string, unknown> = {};
    if (workflowId !== undefined) params.skill_id = workflowId;
    if (limit !== undefined) params.limit = limit;
    const response = await callCoreRpc<Envelope<{ runs: ScannedRun[] }> | { runs: ScannedRun[] }>({
      method: 'openhuman.skill_runtime_recent_runs',
      params,
    });
    const raw = unwrapEnvelope(response);
    log('recentRuns: response count=%d', raw.runs.length);
    return raw.runs;
  },

  /**
   * Resolve the reusable Node/Python runtimes backing script-based skills.
   * The backend reuses `runtime_node` and `runtime_python`; this call is a
   * cheap UI/prod-smoke probe unless it has to bootstrap a missing managed runtime.
   */
  resolveRuntimes: async (
    runtime: 'all' | 'node' | 'python' = 'all'
  ): Promise<ResolveSkillRuntimesResult> => {
    log('resolveRuntimes: request runtime=%s', runtime);
    const response = await callCoreRpc<
      Envelope<RawResolveSkillRuntimesResult> | RawResolveSkillRuntimesResult
    >({
      method: 'openhuman.skill_runtime_resolve_runtimes',
      params: runtime === 'all' ? {} : { runtime },
    });
    const raw = unwrapEnvelope(response);
    const result: ResolveSkillRuntimesResult = {
      runtimes: (raw.runtimes ?? []).map(item => ({
        runtime: item.runtime,
        enabled: item.enabled,
        available: item.available,
        source: item.source,
        version: item.version,
        binary: item.binary,
        binDir: item.bin_dir,
        error: item.error,
      })),
    };
    log('resolveRuntimes: response count=%d', result.runtimes.length);
    return result;
  },
};

/**
 * One input declaration from a skill's `[[inputs]]` block, returned by
 * `openhuman.workflows_describe`. The FE renders one form control per entry:
 * `string`/`integer`/`boolean` map to text/number/checkbox controls.
 */
export interface WorkflowInputDescription {
  name: string;
  description: string;
  required: boolean;
  /** Type hint from `[[inputs]].type`. */
  type: string;
}

/** Wire shape returned by `openhuman.workflows_describe`. */
export interface WorkflowDescription {
  id: string;
  display_name: string;
  when_to_use: string;
  inputs: WorkflowInputDescription[];
}

/** Wire shape returned by `openhuman.skill_runtime_run` (fire-and-forget). */
export interface WorkflowRunStarted {
  run_id: string;
  status: string; // "started"
  workflow_id: string;
  log: string; // absolute path to the streaming log
}

interface RawSkillRunStarted {
  run_id: string;
  status: string;
  workflow_id?: string;
  skill_id: string;
  log: string;
}

/**
 * Slice of a run log file returned by `openhuman.skill_runtime_read_run_log`.
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
 * One run entry returned by `openhuman.skill_runtime_recent_runs`. Wire shape
 * mirrors `crate::openhuman::skills::run_log::ScannedRun`. `status` is
 * `"RUNNING"` while the run hasn't written its `--- result ---` footer
 * yet; after the footer lands it becomes `"DONE"` / `"DEGENERATE"` /
 * `"FAILED"`.
 */
export interface ScannedRun {
  run_id: string;
  workflow_id: string;
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
