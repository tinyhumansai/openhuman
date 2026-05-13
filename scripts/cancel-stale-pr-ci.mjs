#!/usr/bin/env node

import { execFileSync } from 'node:child_process';

const DEFAULT_REPO = 'tinyhumansai/openhuman';
const DEFAULT_MAX_AGE_MINUTES = 20;
const DEFAULT_OPEN_PR_LIMIT = 200;
const DEFAULT_EXCLUDE_WORKFLOW_PATTERNS = ['release'];
const ACTIVE_RUN_STATUSES = new Set(['queued', 'in_progress', 'pending', 'requested', 'waiting']);

function printUsage() {
  console.log(`Usage: cancel-stale-pr-ci.mjs [options]

Scan open pull requests, find non-release GitHub Actions runs older than the
age threshold, and cancel them.

Options:
  --repo <owner/name>           Repository to inspect (default: ${DEFAULT_REPO})
  --max-age-minutes <minutes>   Cancel runs older than this (default: ${DEFAULT_MAX_AGE_MINUTES})
  --open-pr-limit <count>       Max open PRs to inspect (default: ${DEFAULT_OPEN_PR_LIMIT})
  --exclude-workflow <pattern>  Case-insensitive substring/regex fragment to skip.
                                May be passed multiple times. Default: ${DEFAULT_EXCLUDE_WORKFLOW_PATTERNS.join(', ')}
  --execute                     Actually cancel matching runs.
  --help                        Show this message.

Examples:
  node scripts/cancel-stale-pr-ci.mjs
  node scripts/cancel-stale-pr-ci.mjs --execute
  node scripts/cancel-stale-pr-ci.mjs --execute --exclude-workflow release --exclude-workflow staging
`);
}

function fail(message) {
  console.error(`[ci-cleanup] ${message}`);
  process.exit(1);
}

function runGhJson(args) {
  const stdout = execFileSync('gh', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  return JSON.parse(stdout);
}

function runGh(args) {
  return execFileSync('gh', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
}

function parseArgs(argv) {
  const options = {
    repo: DEFAULT_REPO,
    maxAgeMinutes: DEFAULT_MAX_AGE_MINUTES,
    openPrLimit: DEFAULT_OPEN_PR_LIMIT,
    excludeWorkflowPatterns: [...DEFAULT_EXCLUDE_WORKFLOW_PATTERNS],
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
    if (arg === '--repo') {
      options.repo = argv[++i] ?? fail('Missing value for --repo');
      continue;
    }
    if (arg === '--max-age-minutes') {
      options.maxAgeMinutes = Number(argv[++i]);
      continue;
    }
    if (arg === '--open-pr-limit') {
      options.openPrLimit = Number(argv[++i]);
      continue;
    }
    if (arg === '--exclude-workflow') {
      const pattern = argv[++i];
      if (!pattern) {
        fail('Missing value for --exclude-workflow');
      }
      options.excludeWorkflowPatterns.push(pattern);
      continue;
    }
    fail(`Unknown argument: ${arg}`);
  }

  if (!Number.isFinite(options.maxAgeMinutes) || options.maxAgeMinutes <= 0) {
    fail('--max-age-minutes must be a positive number');
  }
  if (!Number.isInteger(options.openPrLimit) || options.openPrLimit <= 0) {
    fail('--open-pr-limit must be a positive integer');
  }

  return options;
}

function buildExcludeRegexes(patterns) {
  return patterns.map((pattern) => new RegExp(pattern, 'i'));
}

function matchesExcludedWorkflow(run, excludeRegexes) {
  const haystacks = [run.name ?? '', run.path ?? '', run.display_title ?? ''];
  return excludeRegexes.some((regex) => haystacks.some((value) => regex.test(value)));
}

function formatMinutes(minutes) {
  return `${minutes.toFixed(1)}m`;
}

function getOpenPullRequests(repo, limit) {
  return runGhJson([
    'pr',
    'list',
    '--repo',
    repo,
    '--state',
    'open',
    '--limit',
    String(limit),
    '--json',
    'number,title,headRefName,headRefOid,url',
  ]);
}

function getWorkflowRuns(repo, status) {
  const response = runGhJson([
    'api',
    `repos/${repo}/actions/runs?event=pull_request&status=${encodeURIComponent(status)}&per_page=100`,
  ]);
  return response.workflow_runs ?? [];
}

function getRunJobs(repo, runId) {
  const response = runGhJson([
    'api',
    `repos/${repo}/actions/runs/${runId}/jobs?per_page=100`,
  ]);
  return response.jobs ?? [];
}

function cancelRun(repo, runId) {
  try {
    runGh(['api', '--method', 'POST', `repos/${repo}/actions/runs/${runId}/force-cancel`]);
    return 'force-cancel';
  } catch {
    runGh(['api', '--method', 'POST', `repos/${repo}/actions/runs/${runId}/cancel`]);
    return 'cancel';
  }
}

function summarizeActiveJobs(jobs) {
  return jobs
    .filter((job) => ACTIVE_RUN_STATUSES.has(job.status))
    .map((job) => job.name)
    .sort();
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const excludeRegexes = buildExcludeRegexes(options.excludeWorkflowPatterns);
  const now = Date.now();

  console.log(
    `[ci-cleanup] repo=${options.repo} mode=${options.execute ? 'execute' : 'dry-run'} maxAge=${options.maxAgeMinutes}m`,
  );
  console.log(
    `[ci-cleanup] excluding workflows matching: ${options.excludeWorkflowPatterns.join(', ')}`,
  );

  const prs = getOpenPullRequests(options.repo, options.openPrLimit);
  const prsByHeadSha = new Map(prs.map((pr) => [pr.headRefOid, pr]));
  const seenRunIds = new Set();
  const candidates = [];

  for (const status of ACTIVE_RUN_STATUSES) {
    const runs = getWorkflowRuns(options.repo, status);
    for (const run of runs) {
      if (seenRunIds.has(run.id)) {
        continue;
      }
      seenRunIds.add(run.id);

      const pr = prsByHeadSha.get(run.head_sha);
      if (!pr) {
        continue;
      }

      if (!ACTIVE_RUN_STATUSES.has(run.status)) {
        continue;
      }
      if (matchesExcludedWorkflow(run, excludeRegexes)) {
        continue;
      }

      const startedAt = run.run_started_at ?? run.created_at;
      if (!startedAt) {
        continue;
      }
      const ageMinutes = (now - Date.parse(startedAt)) / 60000;
      if (!Number.isFinite(ageMinutes) || ageMinutes < options.maxAgeMinutes) {
        continue;
      }

      const jobs = getRunJobs(options.repo, run.id);
      const activeJobs = summarizeActiveJobs(jobs);

      candidates.push({
        pr,
        run,
        ageMinutes,
        activeJobs,
      });
    }
  }

  if (candidates.length === 0) {
    console.log('[ci-cleanup] no stale PR workflow runs matched');
    return;
  }

  candidates.sort((a, b) => b.ageMinutes - a.ageMinutes);

  for (const candidate of candidates) {
    const { pr, run, ageMinutes, activeJobs } = candidate;
    const jobsLabel = activeJobs.length > 0 ? activeJobs.join(', ') : 'unknown active jobs';
    console.log(
      `[ci-cleanup] candidate pr=#${pr.number} run=${run.id} workflow="${run.name}" status=${run.status} age=${formatMinutes(ageMinutes)} jobs=[${jobsLabel}] url=${run.html_url}`,
    );
  }

  if (!options.execute) {
    console.log('[ci-cleanup] dry-run only; re-run with --execute to cancel these runs');
    return;
  }

  let cancelled = 0;
  for (const candidate of candidates) {
    const method = cancelRun(options.repo, candidate.run.id);
    cancelled += 1;
    console.log(
      `[ci-cleanup] cancelled pr=#${candidate.pr.number} run=${candidate.run.id} workflow="${candidate.run.name}" via ${method}`,
    );
  }

  console.log(`[ci-cleanup] cancelled ${cancelled} stale workflow run(s)`);
}

main();
