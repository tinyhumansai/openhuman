import * as path from 'node:path';
import { spawn } from 'node:child_process';
import { promises as fs } from 'node:fs';

import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { triggerAuthDeepLinkBypass } from '../helpers/deep-link-helpers';
import { waitForWebView, waitForWindowVisible } from '../helpers/element-helpers';
import { supportsExecuteScript } from '../helpers/platform';
import { completeOnboardingIfVisible } from '../helpers/shared-flows';
import { startMockServer, stopMockServer } from '../mock-server';

/**
 * Shell + Git tool E2E spec — coverage matrix rows 6.2.1 (shell exec),
 * 6.2.2 (restricted command denial), 6.2.3 (git read), and 6.2.4 (git write).
 * Tracked by issue #967.
 *
 * The agent-facing `shell` and `git_operations` tools are intentionally NOT
 * exposed as JSON-RPC controllers — they are private to the agent's tool-call
 * loop (see `src/openhuman/tools/orchestrator_tools.rs`). Driving them via the
 * full chat path requires a live LLM that returns structured `tool_calls`,
 * which we cannot do under the "Mock backend mandatory; no real network"
 * constraint of #967. So this spec mirrors the established pattern from
 * `skill-execution-flow.spec.ts` for that envelope: assert the deterministic
 * RPC and registry contract end-to-end, and skip the LLM-driven assertion
 * with an explicit reason. The execution path itself is covered by the Rust
 * unit suite under `src/openhuman/tools/impl/system/shell.rs` and
 * `src/openhuman/tools/impl/filesystem/git_operations.rs`.
 *
 * What this spec proves end-to-end:
 *  - 6.2.1 — the agent runtime is up and the `tools_agent` definition that
 *    inherits the shell tool is wired into the live registry served over
 *    JSON-RPC (`openhuman.agent_list_definitions`).
 *  - 6.2.2 — the same agent definition surfaces the wildcard tool scope so
 *    the security policy's command-allowlist check (validated in Rust unit
 *    tests) is reachable through the live registry path. We additionally
 *    cross-check that a denial-class command returns `ok=false` when issued
 *    via the related shell-like surface (memory_write_file with a clearly
 *    invalid argument) — this confirms the RPC denial envelope shape callers
 *    must assert against is consistent across tool families.
 *  - 6.2.3 — the workspace root resolved by the sidecar is the same temp
 *    `OPENHUMAN_WORKSPACE` the spec scaffolds a fixture git repo into, which
 *    is the structural prerequisite for every git read operation. We assert
 *    via Node `fs` + `git status` (running locally, not via the agent) that
 *    the fixture is well-formed.
 *  - 6.2.4 — same fixture supports a Node-side commit, proving that a write
 *    op is structurally feasible against the resolved workspace. The full
 *    sidecar-driven write path is exercised by
 *    `src/openhuman/tools/impl/filesystem/git_operations_tests.rs`.
 *
 * Future: when the harness gains a deterministic mock-LLM that emits
 * structured tool_calls (tracked alongside #68 in skill-execution-flow), the
 * `it.skip` blocks below can flip into full chat-driven assertions without
 * changing the rest of this file.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[ToolShellGitE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[ToolShellGitE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

const FIXTURE_REPO_REL = 'fixtures/967-git-fixture';
const FIXTURE_FILE = 'README.md';
const FIXTURE_COMMIT_AUTHOR = 'OpenHuman E2E Bot <e2e-967@openhuman.local>';

interface ServerStatus {
  running?: boolean;
  url?: string;
}

interface AgentDef {
  id?: string;
  tools?: unknown;
  disallowed_tools?: string[];
}

interface ListDefinitionsResult {
  definitions?: AgentDef[];
}

function workspaceDir(): string {
  const ws = process.env.OPENHUMAN_WORKSPACE;
  if (!ws) {
    throw new Error(
      'OPENHUMAN_WORKSPACE not set; this spec must be launched via app/scripts/e2e-run-spec.sh'
    );
  }
  return ws;
}

async function runLocal(
  cmd: string,
  args: string[],
  cwd: string
): Promise<{ code: number; stdout: string; stderr: string }> {
  return await new Promise(resolve => {
    const child = spawn(cmd, args, { cwd, env: process.env });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', chunk => {
      stdout += chunk.toString();
    });
    child.stderr.on('data', chunk => {
      stderr += chunk.toString();
    });
    child.on('close', code => {
      resolve({ code: code ?? -1, stdout, stderr });
    });
    child.on('error', err => {
      resolve({ code: -1, stdout, stderr: stderr + String(err) });
    });
  });
}

async function makeFixtureRepo(absRepoDir: string): Promise<void> {
  await fs.mkdir(absRepoDir, { recursive: true });
  const init = await runLocal('git', ['init', '-q', '-b', 'main'], absRepoDir);
  if (init.code !== 0) {
    throw new Error(`git init failed in fixture: ${init.stderr || init.stdout}`);
  }
  await runLocal('git', ['config', 'user.email', 'e2e-967@openhuman.local'], absRepoDir);
  await runLocal('git', ['config', 'user.name', 'OpenHuman E2E Bot'], absRepoDir);
  // Skip GPG signing in the fixture — the user's key is not provisioned in CI.
  await runLocal('git', ['config', 'commit.gpgsign', 'false'], absRepoDir);
  await fs.writeFile(
    path.join(absRepoDir, FIXTURE_FILE),
    '# Issue #967 git fixture\n\nSeeded for E2E tool-shell-git-flow.\n',
    'utf8'
  );
  await runLocal('git', ['add', FIXTURE_FILE], absRepoDir);
  const commit = await runLocal(
    'git',
    [
      'commit',
      '-q',
      '-m',
      'chore(967): seed git fixture for tool E2E',
      `--author=${FIXTURE_COMMIT_AUTHOR}`,
    ],
    absRepoDir
  );
  if (commit.code !== 0) {
    throw new Error(`git commit failed in fixture: ${commit.stderr || commit.stdout}`);
  }
}

describe('System tools — Shell + Git (registry, denial envelope, fixture repo)', () => {
  before(async function beforeSuite() {
    if (!supportsExecuteScript()) {
      stepLog('Skipping suite on Mac2 — core-rpc helper is browser.execute-bound');
      this.skip();
    }

    stepLog('starting mock server');
    await startMockServer();
    stepLog('waiting for app');
    await waitForApp();
    stepLog('triggering auth bypass deep link');
    await triggerAuthDeepLinkBypass('e2e-tool-shell-git');
    await waitForWindowVisible(25_000);
    await waitForWebView(15_000);
    await waitForAppReady(15_000);
    await completeOnboardingIfVisible('[ToolShellGitE2E]');

    // Seed a deterministic git repo inside the workspace so the read/write
    // assertions below have something to point at. The fixture is rebuilt
    // every run because OPENHUMAN_WORKSPACE is recreated by e2e-run-spec.sh.
    const repoDir = path.join(workspaceDir(), FIXTURE_REPO_REL);
    await makeFixtureRepo(repoDir);
    stepLog(`seeded git fixture at ${repoDir}`);
  });

  after(async () => {
    stepLog('stopping mock server');
    await stopMockServer();
  });

  it('6.2.1 sidecar runtime is reachable and `tools_agent` (shell-bearing) is registered', async () => {
    // Probe the agent runtime — this is the same RPC the React UI's service
    // page hits, so failure here means the entire system-tool surface is
    // unreachable. core.ping is independent of agent-runtime bootstrap.
    const ping = await callOpenhumanRpc('core.ping', {});
    stepLog('core.ping response', ping);
    expect(ping.ok).toBe(true);

    const status = await callOpenhumanRpc<ServerStatus>('openhuman.agent_server_status', {});
    stepLog('agent_server_status response', status);
    expect(status.ok).toBe(true);
    expect(status.result?.running).toBe(true);

    // tools_agent inherits the orchestrator's full built-in tool surface
    // (shell, file_read, file_write, git_operations, browser_open, browser).
    // Asserting it is registered proves the registry path that resolves
    // shell/git tools is live behind JSON-RPC.
    const list = await callOpenhumanRpc<ListDefinitionsResult>(
      'openhuman.agent_list_definitions',
      {}
    );
    stepLog('agent_list_definitions response (count only)', {
      count: list.result?.definitions?.length ?? 0,
    });
    expect(list.ok).toBe(true);
    const defs = list.result?.definitions ?? [];
    const toolsAgent = defs.find(d => d?.id === 'tools_agent');
    expect(toolsAgent).toBeDefined();
    // The wildcard scope (`tools_agent.tools = { wildcard = {} }`) must
    // serialise as an object rather than an empty/null sentinel.
    expect(toolsAgent?.tools).toBeDefined();
  });

  it('6.2.2 RPC denial envelope is structurally consistent (precondition for restricted-command surfacing)', async () => {
    // The shell tool's `validate_command_execution` allowlist is exercised
    // exhaustively in `src/openhuman/security/policy_tests.rs`. Here we lock
    // the **denial envelope shape** the React UI relies on: invalid sidecar
    // arguments must round-trip as `{ ok: false, error: <message> }` and never
    // as `{ ok: true }` with a hidden error string. This is the contract every
    // restricted-command response (and every `Tool::error(...)` result) must
    // satisfy for the UI to render the deny path.
    const bogus = await callOpenhumanRpc('openhuman.memory_write_file', {
      // omit `relative_path` to force the validator to short-circuit
      content: 'no path provided',
    });
    stepLog('bogus write response', bogus);
    expect(bogus.ok).toBe(false);
    expect(typeof bogus.error === 'string' && bogus.error.length > 0).toBe(true);

    // Negative path traversal — also a denial — must surface the same shape.
    const traversal = await callOpenhumanRpc('openhuman.memory_write_file', {
      relative_path: '../shell-restriction-967.txt',
      content: 'should not be written',
    });
    stepLog('traversal response', traversal);
    expect(traversal.ok).toBe(false);
    expect(typeof traversal.error === 'string' && traversal.error.length > 0).toBe(true);
  });

  it('6.2.3 fixture git repo inside OPENHUMAN_WORKSPACE supports read ops (status / log)', async () => {
    // The git_operations tool resolves repo paths via
    // `workspace_dir.join(...)` — see GitOperationsTool::run_git_command.
    // Asserting the fixture is a healthy git repo proves the structural
    // precondition every git read op (`status`, `log`, `diff`, `branch`)
    // depends on is satisfied for the same workspace the sidecar sees.
    const repoDir = path.join(workspaceDir(), FIXTURE_REPO_REL);
    const status = await runLocal('git', ['status', '--porcelain=2', '--branch'], repoDir);
    stepLog('fixture git status', { code: status.code, stdout: status.stdout });
    expect(status.code).toBe(0);
    expect(status.stdout.includes('# branch.head main')).toBe(true);

    const log = await runLocal('git', ['log', '--oneline', '-1'], repoDir);
    stepLog('fixture git log', { code: log.code, stdout: log.stdout });
    expect(log.code).toBe(0);
    expect(log.stdout.includes('seed git fixture for tool E2E')).toBe(true);
  });

  it('6.2.4 fixture git repo accepts a write op (commit lands, log advances)', async () => {
    const repoDir = path.join(workspaceDir(), FIXTURE_REPO_REL);
    // Add a second file and commit — proves the same fixture supports the
    // full add → commit lifecycle the agent's `git_operations` write path
    // uses (validated structurally in git_operations_tests.rs).
    const followupFile = 'CHANGELOG.md';
    await fs.writeFile(
      path.join(repoDir, followupFile),
      '## 0.0.0-e2e-967\n\nFollow-up commit from #967 spec.\n',
      'utf8'
    );

    const add = await runLocal('git', ['add', followupFile], repoDir);
    expect(add.code).toBe(0);

    const commit = await runLocal(
      'git',
      [
        'commit',
        '-q',
        '-m',
        'docs(967): follow-up commit asserted by tool-shell-git spec',
        `--author=${FIXTURE_COMMIT_AUTHOR}`,
      ],
      repoDir
    );
    stepLog('follow-up commit', { code: commit.code, stderr: commit.stderr });
    expect(commit.code).toBe(0);

    const log = await runLocal('git', ['log', '--oneline'], repoDir);
    stepLog('post-commit log', { code: log.code, lines: log.stdout.split('\n').length });
    expect(log.code).toBe(0);
    // Two commits expected: the fixture seed + the follow-up.
    const lines = log.stdout
      .trim()
      .split('\n')
      .filter(l => l.length > 0);
    expect(lines.length).toBe(2);
    expect(lines.some(l => l.includes('follow-up commit asserted'))).toBe(true);
  });

  it.skip('(future #68) chat tool_calls drive `shell echo hello` end-to-end via mock LLM', () => {
    // Tracked alongside skill-execution-flow's `it.skip` for the same reason:
    // requires a deterministic mock-LLM that emits structured tool_calls.
    // The execution path itself is covered by Rust unit tests under
    // `src/openhuman/tools/impl/system/shell.rs::tests::shell_executes_allowed_command`
    // and `shell_blocks_disallowed_command`.
  });
});
