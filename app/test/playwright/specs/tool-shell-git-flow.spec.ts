import * as path from 'node:path';
import { expect, test } from '@playwright/test';
import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { promises as fs } from 'node:fs';

import { bootAuthenticatedPage, callCoreRpc } from '../helpers/core-rpc';

const FIXTURE_REPO_REL = 'fixtures/967-git-fixture';
const FIXTURE_FILE = 'README.md';
const FIXTURE_COMMIT_AUTHOR = 'OpenHuman E2E Bot <e2e-967@openhuman.local>';

interface ServerStatus {
  running?: boolean;
  url?: string;
}

function unwrapStatus(raw: unknown): ServerStatus {
  const root = raw as { result?: ServerStatus } & ServerStatus;
  return root.result ?? root;
}

interface AgentDef {
  id?: string;
  tools?: unknown;
  direct_tool_names?: string[];
  disallowed_tools?: string[];
}

interface ListDefinitionsResult {
  definitions?: AgentDef[];
}

function workspaceDir(): string {
  const ws = process.env.OPENHUMAN_WORKSPACE;
  if (!ws) {
    throw new Error('OPENHUMAN_WORKSPACE not set for tool-shell-git-flow Playwright run');
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
  await runLocal('git', ['config', 'commit.gpgsign', 'false'], absRepoDir);
  await fs.writeFile(
    path.join(absRepoDir, FIXTURE_FILE),
    '# Issue #967 git fixture\n\nSeeded for Playwright tool-shell-git-flow.\n',
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

test.describe('System tools - Shell + Git', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-tool-shell-git-' + testSlug, '/home');

    const repoDir = path.join(workspaceDir(), FIXTURE_REPO_REL);
    await fs.rm(repoDir, { recursive: true, force: true });
    await makeFixtureRepo(repoDir);
  });

  test('sidecar runtime is reachable and the orchestrator carries the shell/git tools', async () => {
    const ping = await callCoreRpc<{ ok?: boolean }>('core.ping', {});
    expect(ping.ok).toBe(true);

    const status = unwrapStatus(await callCoreRpc<unknown>('openhuman.agent_server_status', {}));
    expect(status.running).toBe(true);

    const list = await callCoreRpc<ListDefinitionsResult>('openhuman.agent_list_definitions', {});
    const defs = list.definitions ?? [];
    const orchestrator = defs.find(def => def?.id === 'orchestrator');
    expect(orchestrator).toBeDefined();
    const direct = orchestrator?.direct_tool_names ?? [];
    for (const tool of ['shell', 'git_operations', 'file_write']) {
      expect(direct).toContain(tool);
    }
    expect(defs.find(def => def?.id === 'tools_agent')).toBeUndefined();
  });

  test('memory brain ingest rejects missing and conflicting inputs', async () => {
    await expect(callCoreRpc('openhuman.memory_brain_ingest', {})).rejects.toThrow(
      /exactly one of `path` or `text`/i
    );

    await expect(
      callCoreRpc('openhuman.memory_brain_ingest', { path: 'README.md', text: 'conflicting input' })
    ).rejects.toThrow(/exactly one of `path` or `text`/i);
  });

  test('workspace file writes reject traversal and absolute paths without writing outside the workspace', async () => {
    const escapedFile = path.resolve(workspaceDir(), '..', `e2e-escape-967-${randomUUID()}.txt`);
    const traversalName = path.relative(workspaceDir(), escapedFile);
    for (const filename of [traversalName, escapedFile]) {
      await expect(
        callCoreRpc('openhuman.workspace_file_write', {
          filename,
          contents: 'must stay inside the workspace',
        })
      ).rejects.toThrow();
    }
    await expect(fs.readFile(escapedFile, 'utf8')).rejects.toMatchObject({ code: 'ENOENT' });
  });

  test('fixture git repo inside OPENHUMAN_WORKSPACE supports read ops', async () => {
    const repoDir = path.join(workspaceDir(), FIXTURE_REPO_REL);
    const status = await runLocal('git', ['status', '--porcelain=2', '--branch'], repoDir);
    expect(status.code).toBe(0);
    expect(status.stdout.includes('# branch.head main')).toBe(true);

    const log = await runLocal('git', ['log', '--oneline', '-1'], repoDir);
    expect(log.code).toBe(0);
    expect(log.stdout.includes('seed git fixture for tool E2E')).toBe(true);
  });

  test('fixture git repo accepts a write op and log advances', async () => {
    const repoDir = path.join(workspaceDir(), FIXTURE_REPO_REL);
    const followupFile = 'CHANGELOG.md';
    await fs.writeFile(
      path.join(repoDir, followupFile),
      '## 0.0.0-e2e-967\n\nFollow-up commit from Playwright tool-shell-git spec.\n',
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
    expect(commit.code).toBe(0);

    const log = await runLocal('git', ['log', '--oneline'], repoDir);
    expect(log.code).toBe(0);
    const lines = log.stdout
      .trim()
      .split('\n')
      .filter(line => line.length > 0);
    expect(lines.length).toBe(2);
    expect(lines.some(line => line.includes('follow-up commit asserted'))).toBe(true);
  });

});
