#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const DEFAULT_REPO = 'tinyhumansai/openhuman';
const DEFAULT_CHUNK_SIZE = 12;
const DEFAULT_PROMPT_BODY_CHARS = 3200;
const DEFAULT_LLM = 'codex';
const DEFAULT_OUT_ROOT = 'tmp/test-planning';

function printUsage() {
  console.log(`Usage: build-github-test-map.mjs [options]

Fetch GitHub issues / PRs, synthesize candidate test coverage targets with an
LLM, then emit both JSONL and Markdown outputs.

Options:
  --repo <owner/name>            Repository to inspect (default: ${DEFAULT_REPO})
  --phase <all|fetch|synthesize> Which phase to run (default: all)
  --include <issues|prs|both>    Which sources to fetch (default: both)
  --out-dir <path>               Output directory (default: ${DEFAULT_OUT_ROOT}/<timestamp>)
  --updated-since <ISO date>     Keep only items updated on/after this instant
  --max-issues <count>           Cap fetched issues (0 = all, default: 0)
  --max-prs <count>              Cap fetched pull requests (0 = all, default: 0)
  --chunk-size <count>           Source items per synthesis batch (default: ${DEFAULT_CHUNK_SIZE})
  --prompt-body-chars <count>    Max body chars per item sent to the LLM (default: ${DEFAULT_PROMPT_BODY_CHARS})
  --llm <codex|claude>           Non-interactive CLI to use for synthesis (default: ${DEFAULT_LLM})
  --model <name>                 Optional model override for the LLM CLI
  --help                         Show this message

Examples:
  node scripts/test-planning/build-github-test-map.mjs --max-prs 50 --max-issues 50
  node scripts/test-planning/build-github-test-map.mjs --phase fetch --updated-since 2025-01-01T00:00:00Z
  node scripts/test-planning/build-github-test-map.mjs --phase synthesize --out-dir tmp/test-planning/20260514T120000Z
`);
}

function fail(message) {
  console.error(`[test-plan] ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const options = {
    repo: DEFAULT_REPO,
    phase: 'all',
    include: 'both',
    outDir: path.join(DEFAULT_OUT_ROOT, makeTimestamp()),
    updatedSince: null,
    maxIssues: 0,
    maxPrs: 0,
    chunkSize: DEFAULT_CHUNK_SIZE,
    promptBodyChars: DEFAULT_PROMPT_BODY_CHARS,
    llm: DEFAULT_LLM,
    model: null,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--') {
      continue;
    }
    if (arg === '--help') {
      printUsage();
      process.exit(0);
    }
    if (arg === '--repo') {
      options.repo = argv[++i] ?? fail('Missing value for --repo');
      continue;
    }
    if (arg === '--phase') {
      options.phase = argv[++i] ?? fail('Missing value for --phase');
      continue;
    }
    if (arg === '--include') {
      options.include = argv[++i] ?? fail('Missing value for --include');
      continue;
    }
    if (arg === '--out-dir') {
      options.outDir = argv[++i] ?? fail('Missing value for --out-dir');
      continue;
    }
    if (arg === '--updated-since') {
      options.updatedSince = argv[++i] ?? fail('Missing value for --updated-since');
      continue;
    }
    if (arg === '--max-issues') {
      options.maxIssues = Number(argv[++i]);
      continue;
    }
    if (arg === '--max-prs') {
      options.maxPrs = Number(argv[++i]);
      continue;
    }
    if (arg === '--chunk-size') {
      options.chunkSize = Number(argv[++i]);
      continue;
    }
    if (arg === '--prompt-body-chars') {
      options.promptBodyChars = Number(argv[++i]);
      continue;
    }
    if (arg === '--llm') {
      options.llm = argv[++i] ?? fail('Missing value for --llm');
      continue;
    }
    if (arg === '--model') {
      options.model = argv[++i] ?? fail('Missing value for --model');
      continue;
    }
    fail(`Unknown argument: ${arg}`);
  }

  if (!['all', 'fetch', 'synthesize'].includes(options.phase)) {
    fail(`Unsupported --phase: ${options.phase}`);
  }
  if (!['issues', 'prs', 'both'].includes(options.include)) {
    fail(`Unsupported --include: ${options.include}`);
  }
  if (!['codex', 'claude'].includes(options.llm)) {
    fail(`Unsupported --llm: ${options.llm}`);
  }
  if (!Number.isInteger(options.maxIssues) || options.maxIssues < 0) {
    fail('--max-issues must be a non-negative integer');
  }
  if (!Number.isInteger(options.maxPrs) || options.maxPrs < 0) {
    fail('--max-prs must be a non-negative integer');
  }
  if (!Number.isInteger(options.chunkSize) || options.chunkSize <= 0) {
    fail('--chunk-size must be a positive integer');
  }
  if (!Number.isInteger(options.promptBodyChars) || options.promptBodyChars <= 0) {
    fail('--prompt-body-chars must be a positive integer');
  }
  if (options.updatedSince) {
    const timestamp = Date.parse(options.updatedSince);
    if (Number.isNaN(timestamp)) {
      fail(`Invalid --updated-since value: ${options.updatedSince}`);
    }
  }

  return options;
}

function makeTimestamp() {
  return new Date().toISOString().replace(/[-:]/g, '').replace(/\.\d{3}Z$/, 'Z');
}

function ensureDir(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
}

function run(command, args, options = {}) {
  return execFileSync(command, args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    ...options,
  });
}

function runJson(command, args, options = {}) {
  return JSON.parse(run(command, args, options));
}

function splitRepo(repo) {
  const [owner, name] = repo.split('/');
  if (!owner || !name) {
    fail(`Invalid repo slug: ${repo}`);
  }
  return { owner, name };
}

function graphQl(query, variables) {
  const args = ['api', 'graphql', '-f', `query=${query}`];
  for (const [key, value] of Object.entries(variables)) {
    if (value === null || value === undefined) {
      continue;
    }
    args.push('-F', `${key}=${value}`);
  }
  return runJson('gh', args);
}

function truncateBody(body, maxChars) {
  const normalized = (body ?? '').replace(/\r\n/g, '\n').trim();
  if (normalized.length <= maxChars) {
    return normalized;
  }
  return `${normalized.slice(0, maxChars)}\n...[truncated]`;
}

function compactWhitespace(value) {
  return (value ?? '').replace(/\s+/g, ' ').trim();
}

function normalizeLabels(labelNodes) {
  return (labelNodes ?? []).map((node) => node.name).filter(Boolean);
}

function normalizeIssue(node) {
  return {
    kind: 'issue',
    source_id: `issue#${node.number}`,
    number: node.number,
    title: node.title,
    body: node.body ?? '',
    url: node.url,
    state: String(node.state ?? '').toLowerCase(),
    created_at: node.createdAt,
    updated_at: node.updatedAt,
    closed_at: node.closedAt,
    merged_at: null,
    labels: normalizeLabels(node.labels?.nodes),
    author: node.author?.login ?? null,
  };
}

function normalizePr(node) {
  return {
    kind: 'pr',
    source_id: `pr#${node.number}`,
    number: node.number,
    title: node.title,
    body: node.body ?? '',
    url: node.url,
    state: String(node.state ?? '').toLowerCase(),
    created_at: node.createdAt,
    updated_at: node.updatedAt,
    closed_at: node.closedAt,
    merged_at: node.mergedAt,
    labels: normalizeLabels(node.labels?.nodes),
    author: node.author?.login ?? null,
    is_draft: Boolean(node.isDraft),
    changed_files: node.changedFiles ?? null,
    additions: node.additions ?? null,
    deletions: node.deletions ?? null,
    base_ref: node.baseRefName ?? null,
    head_ref: node.headRefName ?? null,
  };
}

function filterItems(items, updatedSince) {
  if (!updatedSince) {
    return items;
  }
  const cutoff = Date.parse(updatedSince);
  return items.filter((item) => Date.parse(item.updated_at) >= cutoff);
}

function fetchIssues(repo, maxItems) {
  const { owner, name } = splitRepo(repo);
  const query = `
    query($owner: String!, $name: String!, $cursor: String) {
      repository(owner: $owner, name: $name) {
        issues(first: 100, after: $cursor, orderBy: { field: UPDATED_AT, direction: DESC }, states: [OPEN, CLOSED]) {
          pageInfo { hasNextPage endCursor }
          nodes {
            number
            title
            body
            url
            state
            createdAt
            updatedAt
            closedAt
            author { login }
            labels(first: 20) { nodes { name } }
          }
        }
      }
    }
  `;

  const items = [];
  let cursor = null;
  while (true) {
    const response = graphQl(query, { owner, name, cursor });
    const connection = response.data?.repository?.issues;
    const nodes = connection?.nodes ?? [];
    for (const node of nodes) {
      items.push(normalizeIssue(node));
      if (maxItems > 0 && items.length >= maxItems) {
        return items;
      }
    }
    if (!connection?.pageInfo?.hasNextPage) {
      return items;
    }
    cursor = connection.pageInfo.endCursor;
  }
}

function fetchPullRequests(repo, maxItems) {
  const { owner, name } = splitRepo(repo);
  const query = `
    query($owner: String!, $name: String!, $cursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequests(first: 100, after: $cursor, orderBy: { field: UPDATED_AT, direction: DESC }, states: [OPEN, CLOSED, MERGED]) {
          pageInfo { hasNextPage endCursor }
          nodes {
            number
            title
            body
            url
            state
            createdAt
            updatedAt
            closedAt
            mergedAt
            isDraft
            changedFiles
            additions
            deletions
            baseRefName
            headRefName
            author { login }
            labels(first: 20) { nodes { name } }
          }
        }
      }
    }
  `;

  const items = [];
  let cursor = null;
  while (true) {
    const response = graphQl(query, { owner, name, cursor });
    const connection = response.data?.repository?.pullRequests;
    const nodes = connection?.nodes ?? [];
    for (const node of nodes) {
      items.push(normalizePr(node));
      if (maxItems > 0 && items.length >= maxItems) {
        return items;
      }
    }
    if (!connection?.pageInfo?.hasNextPage) {
      return items;
    }
    cursor = connection.pageInfo.endCursor;
  }
}

function writeJson(filePath, value) {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
}

function writeJsonAtomic(filePath, value) {
  const tempPath = `${filePath}.tmp`;
  fs.writeFileSync(tempPath, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
  fs.renameSync(tempPath, filePath);
}

function writeJsonl(filePath, rows) {
  const payload = rows.map((row) => JSON.stringify(row)).join('\n');
  fs.writeFileSync(filePath, payload ? `${payload}\n` : '', 'utf8');
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, 'utf8'));
}

function readJsonl(filePath) {
  return fs
    .readFileSync(filePath, 'utf8')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

function fetchPhase(options) {
  ensureDir(options.outDir);
  const fetched = [];

  if (options.include === 'issues' || options.include === 'both') {
    console.error(`[test-plan] fetching issues from ${options.repo}`);
    const issues = filterItems(fetchIssues(options.repo, options.maxIssues), options.updatedSince);
    writeJsonl(path.join(options.outDir, 'raw-issues.jsonl'), issues);
    fetched.push(...issues);
    console.error(`[test-plan] fetched ${issues.length} issues`);
  }

  if (options.include === 'prs' || options.include === 'both') {
    console.error(`[test-plan] fetching pull requests from ${options.repo}`);
    const prs = filterItems(fetchPullRequests(options.repo, options.maxPrs), options.updatedSince);
    writeJsonl(path.join(options.outDir, 'raw-prs.jsonl'), prs);
    fetched.push(...prs);
    console.error(`[test-plan] fetched ${prs.length} pull requests`);
  }

  fetched.sort((a, b) => Date.parse(b.updated_at) - Date.parse(a.updated_at));
  writeJsonl(path.join(options.outDir, 'raw-all.jsonl'), fetched);

  const manifest = {
    generated_at: new Date().toISOString(),
    repo: options.repo,
    include: options.include,
    updated_since: options.updatedSince,
    counts: {
      total: fetched.length,
      issues: fetched.filter((item) => item.kind === 'issue').length,
      prs: fetched.filter((item) => item.kind === 'pr').length,
    },
    prompt_body_chars: options.promptBodyChars,
    chunk_size: options.chunkSize,
    llm: options.llm,
    model: options.model,
  };
  writeJson(path.join(options.outDir, 'manifest.json'), manifest);
  return manifest;
}

function chunk(items, chunkSize) {
  const chunks = [];
  for (let index = 0; index < items.length; index += chunkSize) {
    chunks.push(items.slice(index, index + chunkSize));
  }
  return chunks;
}

function buildSynthesisPrompt(batch, options, batchIndex, batchCount) {
  const sourceText = batch
    .map((item) => {
      const lines = [
        `SOURCE: ${item.source_id}`,
        `KIND: ${item.kind}`,
        `TITLE: ${compactWhitespace(item.title)}`,
        `STATE: ${item.state}`,
        `UPDATED_AT: ${item.updated_at}`,
        `LABELS: ${item.labels.join(', ') || '(none)'}`,
        `URL: ${item.url}`,
      ];

      if (item.kind === 'pr') {
        lines.push(
          `PR_META: draft=${Boolean(item.is_draft)} changed_files=${item.changed_files ?? 'n/a'} additions=${item.additions ?? 'n/a'} deletions=${item.deletions ?? 'n/a'} base=${item.base_ref ?? 'n/a'} head=${item.head_ref ?? 'n/a'}`,
        );
      }

      lines.push(`BODY:\n${truncateBody(item.body, options.promptBodyChars) || '(empty)'}`);
      return lines.join('\n');
    })
    .join('\n\n---\n\n');

  return `You are compressing GitHub issues and pull requests into a canonical test backlog.

Batch ${batchIndex + 1} of ${batchCount}.

Goal:
- Extract product features, user flows, bug-regression scenarios, and integration behaviors that deserve unit tests and/or end-to-end tests.
- Prefer concrete behavior over implementation details.
- Merge duplicates within this batch.
- Ignore pure release chores, repo admin work, formatting-only changes, and vague meta items unless they imply a concrete regression risk.

Output rules:
- Return JSON only that matches the provided schema.
- Emit one item per distinct feature or regression scenario.
- "feature_id" must be stable kebab-case and concise.
- "unit_test_targets" should describe focused logic/component/controller cases.
- "e2e_test_flows" should describe full user-visible or cross-process flows.
- "priority" should reflect regression impact, not effort.
- "confidence" should reflect how strongly the source implies this test target.

Source material:

${sourceText}`;
}

function makeSchema() {
  return {
    type: 'object',
    additionalProperties: false,
    required: ['items'],
    properties: {
      items: {
        type: 'array',
        items: {
          type: 'object',
          additionalProperties: false,
          required: [
            'feature_id',
            'feature_title',
            'feature_summary',
            'source_refs',
            'source_urls',
            'unit_test_targets',
            'e2e_test_flows',
            'regression_risks',
            'priority',
            'confidence',
          ],
          properties: {
            feature_id: { type: 'string' },
            feature_title: { type: 'string' },
            feature_summary: { type: 'string' },
            source_refs: {
              type: 'array',
              items: { type: 'string' },
            },
            source_urls: {
              type: 'array',
              items: { type: 'string' },
            },
            unit_test_targets: {
              type: 'array',
              items: { type: 'string' },
            },
            e2e_test_flows: {
              type: 'array',
              items: { type: 'string' },
            },
            regression_risks: {
              type: 'array',
              items: { type: 'string' },
            },
            priority: {
              type: 'string',
              enum: ['high', 'medium', 'low'],
            },
            confidence: {
              type: 'string',
              enum: ['high', 'medium', 'low'],
            },
          },
        },
      },
    },
  };
}

function invokeCodex(prompt, schema, options) {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'openhuman-test-plan-codex-'));
  const schemaPath = path.join(tempDir, 'schema.json');
  const outputPath = path.join(tempDir, 'output.json');
  fs.writeFileSync(schemaPath, JSON.stringify(schema), 'utf8');

  const args = [
    'exec',
    '--sandbox',
    'read-only',
    '--skip-git-repo-check',
    '--output-schema',
    schemaPath,
    '--output-last-message',
    outputPath,
    '--cd',
    process.cwd(),
    '-',
  ];

  if (options.model) {
    args.splice(1, 0, '--model', options.model);
  }

  execFileSync('codex', args, {
    input: prompt,
    encoding: 'utf8',
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  const payload = fs.readFileSync(outputPath, 'utf8');
  return JSON.parse(payload);
}

function invokeClaude(prompt, schema, options) {
  const args = [
    '-p',
    '--output-format',
    'json',
    '--json-schema',
    JSON.stringify(schema),
    '--permission-mode',
    'default',
  ];

  if (options.model) {
    args.push('--model', options.model);
  }

  args.push(prompt);
  const payload = execFileSync('claude', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  const parsed = JSON.parse(payload);
  if (parsed && typeof parsed === 'object' && parsed.structured_output) {
    return parsed.structured_output;
  }
  return parsed;
}

function invokeLlm(prompt, schema, options) {
  if (options.llm === 'codex') {
    return invokeCodex(prompt, schema, options);
  }
  if (options.llm === 'claude') {
    return invokeClaude(prompt, schema, options);
  }
  fail(`Unsupported LLM: ${options.llm}`);
}

function uniqueStrings(values) {
  return [...new Set(values.map((value) => compactWhitespace(value)).filter(Boolean))];
}

function mergeFeature(existing, next) {
  return {
    ...existing,
    feature_title: existing.feature_title.length >= next.feature_title.length ? existing.feature_title : next.feature_title,
    feature_summary:
      existing.feature_summary.length >= next.feature_summary.length
        ? existing.feature_summary
        : next.feature_summary,
    source_refs: uniqueStrings([...existing.source_refs, ...next.source_refs]),
    source_urls: uniqueStrings([...existing.source_urls, ...next.source_urls]),
    unit_test_targets: uniqueStrings([...existing.unit_test_targets, ...next.unit_test_targets]),
    e2e_test_flows: uniqueStrings([...existing.e2e_test_flows, ...next.e2e_test_flows]),
    regression_risks: uniqueStrings([...existing.regression_risks, ...next.regression_risks]),
    priority: pickPriority(existing.priority, next.priority),
    confidence: pickPriority(existing.confidence, next.confidence),
  };
}

function pickPriority(a, b) {
  const rank = { high: 3, medium: 2, low: 1 };
  return (rank[a] ?? 0) >= (rank[b] ?? 0) ? a : b;
}

function sortFeatures(features) {
  const rank = { high: 0, medium: 1, low: 2 };
  return [...features].sort((a, b) => {
    const priorityDiff = (rank[a.priority] ?? 9) - (rank[b.priority] ?? 9);
    if (priorityDiff !== 0) {
      return priorityDiff;
    }
    return a.feature_title.localeCompare(b.feature_title);
  });
}

function renderMarkdown(features, manifest) {
  const lines = [
    '# GitHub Test Map',
    '',
    `Generated: ${manifest.generated_at}`,
    `Repository: ${manifest.repo}`,
    `Sources: ${manifest.counts.total} items (${manifest.counts.issues} issues, ${manifest.counts.prs} PRs)`,
    `Synthesized features: ${features.length}`,
    `LLM: ${manifest.llm}${manifest.model ? ` (${manifest.model})` : ''}`,
    '',
  ];

  for (const feature of features) {
    lines.push(`## ${feature.feature_title}`);
    lines.push('');
    lines.push(`- Priority: ${feature.priority}`);
    lines.push(`- Confidence: ${feature.confidence}`);
    lines.push(`- Summary: ${feature.feature_summary}`);
    lines.push(`- Sources: ${feature.source_refs.join(', ')}`);
    if (feature.regression_risks.length > 0) {
      lines.push(`- Regression risks: ${feature.regression_risks.join(' | ')}`);
    }
    if (feature.unit_test_targets.length > 0) {
      lines.push('- Unit test targets:');
      for (const target of feature.unit_test_targets) {
        lines.push(`  - ${target}`);
      }
    }
    if (feature.e2e_test_flows.length > 0) {
      lines.push('- E2E test flows:');
      for (const flow of feature.e2e_test_flows) {
        lines.push(`  - ${flow}`);
      }
    }
    lines.push('');
  }

  return `${lines.join('\n').replace(/\n {2}-/g, '\n-')}\n`;
}

function synthesizePhase(options) {
  const rawPath = path.join(options.outDir, 'raw-all.jsonl');
  const manifestPath = path.join(options.outDir, 'manifest.json');
  if (!fs.existsSync(rawPath)) {
    fail(`Missing raw input file: ${rawPath}. Run --phase fetch or --phase all first.`);
  }
  if (!fs.existsSync(manifestPath)) {
    fail(`Missing manifest: ${manifestPath}. Run --phase fetch or --phase all first.`);
  }

  const manifest = readJson(manifestPath);
  manifest.generated_at = new Date().toISOString();
  manifest.llm = options.llm;
  manifest.model = options.model;
  manifest.chunk_size = options.chunkSize;
  manifest.prompt_body_chars = options.promptBodyChars;

  const rawItems = readJsonl(rawPath);
  const batches = chunk(rawItems, options.chunkSize);
  const schema = makeSchema();
  const batchOutputDir = path.join(options.outDir, 'batches');
  ensureDir(batchOutputDir);

  const synthesizedRows = [];
  for (let index = 0; index < batches.length; index += 1) {
    const batchPath = path.join(batchOutputDir, `batch-${String(index + 1).padStart(4, '0')}.json`);
    let response;
    if (fs.existsSync(batchPath)) {
      console.error(`[test-plan] reusing batch ${index + 1}/${batches.length}`);
      response = readJson(batchPath);
    } else {
      const prompt = buildSynthesisPrompt(batches[index], options, index, batches.length);
      console.error(`[test-plan] synthesizing batch ${index + 1}/${batches.length}`);
      response = invokeLlm(prompt, schema, options);
      writeJsonAtomic(batchPath, response);
    }
    for (const item of response?.items ?? []) {
      synthesizedRows.push(item);
    }
  }

  writeJsonl(path.join(options.outDir, 'synthesized-raw.jsonl'), synthesizedRows);

  const mergedById = new Map();
  for (const feature of synthesizedRows) {
    const normalized = {
      feature_id: compactWhitespace(feature.feature_id).toLowerCase(),
      feature_title: compactWhitespace(feature.feature_title),
      feature_summary: compactWhitespace(feature.feature_summary),
      source_refs: uniqueStrings(feature.source_refs ?? []),
      source_urls: uniqueStrings(feature.source_urls ?? []),
      unit_test_targets: uniqueStrings(feature.unit_test_targets ?? []),
      e2e_test_flows: uniqueStrings(feature.e2e_test_flows ?? []),
      regression_risks: uniqueStrings(feature.regression_risks ?? []),
      priority: feature.priority,
      confidence: feature.confidence,
    };
    if (!normalized.feature_id) {
      continue;
    }
    const existing = mergedById.get(normalized.feature_id);
    mergedById.set(normalized.feature_id, existing ? mergeFeature(existing, normalized) : normalized);
  }

  const finalFeatures = sortFeatures([...mergedById.values()]);
  writeJsonl(path.join(options.outDir, 'test-map.jsonl'), finalFeatures);
  fs.writeFileSync(path.join(options.outDir, 'test-map.md'), renderMarkdown(finalFeatures, manifest), 'utf8');
  writeJson(manifestPath, manifest);

  console.error(
    `[test-plan] wrote ${finalFeatures.length} canonical features to ${path.join(options.outDir, 'test-map.jsonl')}`,
  );
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  ensureDir(options.outDir);

  if (options.phase === 'all' || options.phase === 'fetch') {
    fetchPhase(options);
  }

  if (options.phase === 'all' || options.phase === 'synthesize') {
    synthesizePhase(options);
  }
}

main();
