#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const DEFAULT_REPO = 'tinyhumansai/openhuman';
const DEFAULT_BASE_BRANCH = 'main';
const DEFAULT_OPEN_PR_LIMIT = 200;

function printUsage() {
  console.log(`Usage: merge-main-into-open-prs.mjs [options]

Scan open pull requests, check out each head branch in a temporary clone, merge
the latest base branch into it, and push the result back to the same PR branch.
PRs with merge conflicts or push failures are skipped.

Options:
  --repo <owner/name>         Repository to inspect (default: ${DEFAULT_REPO})
  --base-branch <name>        Base branch to merge into PRs (default: ${DEFAULT_BASE_BRANCH})
  --open-pr-limit <count>     Max open PRs to inspect (default: ${DEFAULT_OPEN_PR_LIMIT})
  --pr <number>               Restrict to one PR number. May be passed multiple times.
  --include-drafts            Include draft PRs (default: false)
  --execute                   Actually merge and push. Dry-run by default.
  --help                      Show this message.

Examples:
  node scripts/merge-main-into-open-prs.mjs
  node scripts/merge-main-into-open-prs.mjs --execute
  node scripts/merge-main-into-open-prs.mjs --execute --pr 101 --pr 205
  node scripts/merge-main-into-open-prs.mjs --execute --include-drafts
`);
}

function fail(message) {
  console.error(`[pr-main-sync] ${message}`);
  process.exit(1);
}

function run(command, args, options = {}) {
  const { cwd, stdio = ['ignore', 'pipe', 'pipe'] } = options;
  return execFileSync(command, args, {
    cwd,
    encoding: 'utf8',
    stdio,
  });
}

function runJson(command, args, options = {}) {
  return JSON.parse(run(command, args, options));
}

function tryRun(command, args, options = {}) {
  try {
    return {
      ok: true,
      stdout: run(command, args, options),
    };
  } catch (error) {
    return {
      ok: false,
      error,
      stdout: error.stdout?.toString?.() ?? '',
      stderr: error.stderr?.toString?.() ?? '',
    };
  }
}

function parseArgs(argv) {
  const options = {
    repo: DEFAULT_REPO,
    baseBranch: DEFAULT_BASE_BRANCH,
    openPrLimit: DEFAULT_OPEN_PR_LIMIT,
    prs: [],
    includeDrafts: false,
    execute: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--help') {
      printUsage();
      process.exit(0);
    }
    if (arg === '--execute') {
      options.execute = true;
      continue;
    }
    if (arg === '--include-drafts') {
      options.includeDrafts = true;
      continue;
    }
    if (arg === '--repo') {
      options.repo = argv[++i] ?? fail('Missing value for --repo');
      continue;
    }
    if (arg === '--base-branch') {
      options.baseBranch = argv[++i] ?? fail('Missing value for --base-branch');
      continue;
    }
    if (arg === '--open-pr-limit') {
      options.openPrLimit = Number(argv[++i]);
      continue;
    }
    if (arg === '--pr') {
      const value = argv[++i];
      if (!value) {
        fail('Missing value for --pr');
      }
      const pr = Number(value);
      if (!Number.isInteger(pr) || pr <= 0) {
        fail(`Invalid PR number for --pr: ${value}`);
      }
      options.prs.push(pr);
      continue;
    }
    fail(`Unknown argument: ${arg}`);
  }

  if (!Number.isInteger(options.openPrLimit) || options.openPrLimit <= 0) {
    fail('--open-pr-limit must be a positive integer');
  }

  return options;
}

function getOpenPullRequests(repo, limit) {
  return runJson('gh', [
    'pr',
    'list',
    '--repo',
    repo,
    '--state',
    'open',
    '--limit',
    String(limit),
    '--json',
    'number,title,url,isDraft,baseRefName,headRefName,headRepository,headRepositoryOwner',
  ]);
}

function getPreferredRemoteUrls(repoRoot) {
  const output = run('git', ['remote', '-v'], { cwd: repoRoot });
  const map = new Map();

  for (const line of output.split('\n')) {
    const match = line.match(/^\S+\s+(\S+)\s+\((fetch|push)\)$/);
    if (!match) {
      continue;
    }
    const url = match[1];
    const repo = normalizeRepoFromRemoteUrl(url);
    if (!repo || map.has(repo)) {
      continue;
    }
    map.set(repo, url);
  }

  return map;
}

function normalizeRepoFromRemoteUrl(url) {
  return url
    .replace(/^git@github\.com:/, '')
    .replace(/^https?:\/\/github\.com\//, '')
    .replace(/\.git$/, '');
}

function sanitizeBranchName(branch) {
  return branch.replace(/[^A-Za-z0-9._/-]/g, '-');
}

function ensureCleanBranch(cloneDir, branchName, remoteName, remoteBranch) {
  const localBranch = sanitizeBranchName(branchName);
  run('git', ['checkout', '-B', localBranch, `${remoteName}/${remoteBranch}`], { cwd: cloneDir });
  run('git', ['reset', '--hard', `${remoteName}/${remoteBranch}`], { cwd: cloneDir });
  run('git', ['clean', '-fd'], { cwd: cloneDir });
}

function setupTempClone(repo, baseBranch) {
  const cloneDir = fs.mkdtempSync(path.join(os.tmpdir(), 'openhuman-pr-main-sync-'));
  const repoUrl = `git@github.com:${repo}.git`;

  run('git', ['init'], { cwd: cloneDir });
  run('git', ['remote', 'add', 'upstream', repoUrl], { cwd: cloneDir });
  run('git', ['fetch', '--depth', '50', 'upstream', baseBranch], { cwd: cloneDir });
  run('git', ['checkout', '-B', baseBranch, 'FETCH_HEAD'], { cwd: cloneDir });

  return cloneDir;
}

function getHeadRepoSlug(pr) {
  const owner = pr.headRepositoryOwner?.login;
  const name = pr.headRepository?.name;
  if (!owner || !name) {
    return null;
  }
  return `${owner}/${name}`;
}

function mergeBaseIntoPr({ cloneDir, pr, baseBranch, preferredRemoteUrls, execute }) {
  const headRepo = getHeadRepoSlug(pr);
  if (!headRepo) {
    return {
      status: 'skipped',
      reason: 'missing-head-repo',
    };
  }

  const remoteName = `pr-${pr.number}`;
  const remoteUrl = preferredRemoteUrls.get(headRepo) ?? `git@github.com:${headRepo}.git`;
  const headBranch = pr.headRefName;
  const localBranch = `pr/${pr.number}`;

  const addRemoteResult = tryRun('git', ['remote', 'add', remoteName, remoteUrl], { cwd: cloneDir });
  if (!addRemoteResult.ok && !addRemoteResult.stderr.includes('already exists')) {
    return {
      status: 'skipped',
      reason: 'remote-add-failed',
      detail: addRemoteResult.stderr.trim(),
    };
  }

  run('git', ['fetch', '--depth', '50', 'upstream', baseBranch], { cwd: cloneDir });

  const fetchResult = tryRun(
    'git',
    ['fetch', remoteName, `+refs/heads/${headBranch}:refs/remotes/${remoteName}/${headBranch}`],
    { cwd: cloneDir },
  );
  if (!fetchResult.ok) {
    return {
      status: 'skipped',
      reason: 'head-fetch-failed',
      detail: fetchResult.stderr.trim(),
    };
  }

  ensureCleanBranch(cloneDir, localBranch, remoteName, headBranch);

  const mergeResult = tryRun(
    'git',
    ['merge', '--no-ff', '--no-edit', `upstream/${baseBranch}`],
    { cwd: cloneDir },
  );

  if (!mergeResult.ok) {
    tryRun('git', ['merge', '--abort'], { cwd: cloneDir });
    return {
      status: 'skipped',
      reason: 'merge-conflict',
      detail: mergeResult.stderr.trim() || mergeResult.stdout.trim(),
    };
  }

  const newHead = run('git', ['rev-parse', 'HEAD'], { cwd: cloneDir }).trim();
  const remoteHead = run('git', ['rev-parse', `${remoteName}/${headBranch}`], { cwd: cloneDir }).trim();
  const changed = newHead !== remoteHead;

  if (!changed) {
    return {
      status: 'noop',
      reason: 'already-up-to-date',
    };
  }

  if (!execute) {
    return {
      status: 'dry-run',
      reason: 'would-push',
    };
  }

  const pushResult = tryRun('git', ['push', remoteName, `HEAD:${headBranch}`], { cwd: cloneDir });
  if (!pushResult.ok) {
    return {
      status: 'skipped',
      reason: 'push-failed',
      detail: pushResult.stderr.trim() || pushResult.stdout.trim(),
    };
  }

  return {
    status: 'pushed',
    reason: 'merged-and-pushed',
  };
}

function filterPullRequests(prs, options) {
  const targetPrs = options.prs.length > 0 ? new Set(options.prs) : null;

  return prs.filter((pr) => {
    if (pr.baseRefName !== options.baseBranch) {
      return false;
    }
    if (!options.includeDrafts && pr.isDraft) {
      return false;
    }
    if (targetPrs && !targetPrs.has(pr.number)) {
      return false;
    }
    return true;
  });
}

function summarizeResult(result) {
  if (!result.detail) {
    return result.reason;
  }
  return `${result.reason}: ${result.detail}`;
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const repoRoot = process.cwd();
  const preferredRemoteUrls = getPreferredRemoteUrls(repoRoot);

  console.log(
    `[pr-main-sync] repo=${options.repo} base=${options.baseBranch} mode=${options.execute ? 'execute' : 'dry-run'} limit=${options.openPrLimit}`,
  );
  if (options.prs.length > 0) {
    console.log(`[pr-main-sync] restricted to PRs: ${options.prs.join(', ')}`);
  }

  const prs = filterPullRequests(getOpenPullRequests(options.repo, options.openPrLimit), options);
  if (prs.length === 0) {
    console.log('[pr-main-sync] no matching open PRs found');
    return;
  }

  console.log(`[pr-main-sync] matched ${prs.length} PR(s)`);

  const cloneDir = setupTempClone(options.repo, options.baseBranch);
  const results = [];

  try {
    for (const pr of prs) {
      console.log(
        `[pr-main-sync] processing pr=#${pr.number} branch=${pr.headRefName} url=${pr.url}`,
      );
      const result = mergeBaseIntoPr({
        cloneDir,
        pr,
        baseBranch: options.baseBranch,
        preferredRemoteUrls,
        execute: options.execute,
      });
      results.push({ pr, ...result });
      console.log(
        `[pr-main-sync] result pr=#${pr.number} status=${result.status} ${summarizeResult(result)}`,
      );
    }
  } finally {
    fs.rmSync(cloneDir, { recursive: true, force: true });
  }

  const counts = {
    pushed: 0,
    dryRun: 0,
    noop: 0,
    skipped: 0,
  };

  for (const result of results) {
    if (result.status === 'pushed') {
      counts.pushed += 1;
    } else if (result.status === 'dry-run') {
      counts.dryRun += 1;
    } else if (result.status === 'noop') {
      counts.noop += 1;
    } else {
      counts.skipped += 1;
    }
  }

  console.log(
    `[pr-main-sync] summary pushed=${counts.pushed} dry-run=${counts.dryRun} noop=${counts.noop} skipped=${counts.skipped}`,
  );
}

main();
