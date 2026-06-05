#!/usr/bin/env node

import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

const DEFAULT_MODEL = 'reasoning-v1';
const CORE_BIN = process.env.OPENHUMAN_CORE_BIN || resolve('target/debug/openhuman-core');

const SHARED_REASONING = [
  '# Shared reasoning',
  '- Claims the council agrees on:',
  '- Open disagreements:',
  '- Evidence or constraints to preserve:',
  '- Judge synthesis notes:',
].join('\n');

const seedCouncils = [
  {
    id: 'default-council',
    name: 'Default council',
    description: 'Balanced analyst, builder, and skeptic jury.',
    jury_count: 3,
    debate_rounds: 3,
    seats: [
      seat(0, 'Analyst', 'Evidence, assumptions, and risk.'),
      seat(1, 'Builder', 'Practical implementation path.'),
      seat(2, 'Skeptic', 'Failure modes and missing context.'),
    ],
    judge: judge('Chief Judge'),
    shared_reasoning: SHARED_REASONING,
  },
  {
    id: 'product-review-council',
    name: 'Product Review Council',
    description: 'Evaluates UX, customer impact, and product tradeoffs before shipping.',
    jury_count: 4,
    debate_rounds: 3,
    seats: [
      seat(0, 'User Advocate', 'Protect the user journey, accessibility, and recovery paths.'),
      seat(1, 'Product Strategist', 'Weigh goals, scope, prioritization, and long-term leverage.'),
      seat(2, 'Systems Builder', 'Find the practical implementation path and integration risks.'),
      seat(3, 'Skeptic', 'Challenge weak assumptions, missing evidence, and failure modes.'),
    ],
    judge: judge('Product Judge'),
    shared_reasoning: SHARED_REASONING,
  },
  {
    id: 'architecture-review-council',
    name: 'Architecture Review Council',
    description: 'Reviews technical design, ownership boundaries, performance, and maintainability.',
    jury_count: 4,
    debate_rounds: 3,
    seats: [
      seat(0, 'Domain Owner', 'Keep responsibilities in the right module and surface API contracts.'),
      seat(1, 'Reliability Engineer', 'Focus on failure handling, observability, and deterministic tests.'),
      seat(2, 'Frontend Lead', 'Preserve ergonomic UI flows and avoid duplicated business rules.'),
      seat(3, 'Cost Reviewer', 'Evaluate token, provider, and runtime cost implications.'),
    ],
    judge: judge('Architecture Judge'),
    shared_reasoning: SHARED_REASONING,
  },
  {
    id: 'research-council',
    name: 'Research Council',
    description: 'Separates evidence gathering, source quality, synthesis, and uncertainty.',
    jury_count: 3,
    debate_rounds: 3,
    seats: [
      seat(0, 'Evidence Scout', 'Collect relevant facts, sources, and constraints.'),
      seat(1, 'Methodologist', 'Evaluate source quality and reasoning validity.'),
      seat(2, 'Synthesizer', 'Turn the debate into a concise answer with caveats.'),
    ],
    judge: judge('Research Judge'),
    shared_reasoning: SHARED_REASONING,
  },
];

const options = parseArgs(process.argv.slice(2));

if (options.help) {
  printHelp();
  process.exit(0);
}

if (!existsSync(CORE_BIN) && !options.dryRun) {
  console.error(`Core binary not found: ${CORE_BIN}`);
  console.error('Build it with: cargo build --manifest-path Cargo.toml --bin openhuman-core');
  console.error('Or set OPENHUMAN_CORE_BIN to a compiled openhuman-core binary.');
  process.exit(1);
}

const env = { ...process.env };
if (options.workspace) {
  env.OPENHUMAN_WORKSPACE = options.workspace;
}

if (options.dryRun) {
  console.log(JSON.stringify({ councils: seedCouncils }, null, 2));
  process.exit(0);
}

if (options.replace) {
  const existing = callCore('openhuman.council_registry_list', {}, env);
  const councils = Array.isArray(existing.result) ? existing.result : [];
  for (const council of councils) {
    callCore('openhuman.council_registry_delete', { id: council.id }, env);
  }
}

for (const council of seedCouncils) {
  callCore('openhuman.council_registry_upsert', { council }, env);
  console.log(`Seeded council: ${council.name} (${council.id})`);
}

const workspaceRoot =
  options.workspace || process.env.OPENHUMAN_WORKSPACE || 'the default OpenHuman workspace root';
console.log(`Seeded ${seedCouncils.length} councils through the core registry at ${workspaceRoot}.`);

function seat(id, name, brief) {
  return {
    id,
    mode: 'default',
    profile_id: '',
    name,
    model: DEFAULT_MODEL,
    brief,
  };
}

function judge(name) {
  return {
    mode: 'default',
    profile_id: '',
    name,
    model: DEFAULT_MODEL,
  };
}

function callCore(method, params, env) {
  const child = spawnSync(
    CORE_BIN,
    ['call', '--method', method, '--params', JSON.stringify(params)],
    {
      cwd: resolve('.'),
      env,
      encoding: 'utf8',
    }
  );

  if (child.status !== 0) {
    if (child.stdout.trim()) console.error(child.stdout.trim());
    if (child.stderr.trim()) console.error(child.stderr.trim());
    throw new Error(`openhuman-core call failed for ${method}`);
  }

  const stdout = child.stdout.trim();
  if (!stdout) return {};
  try {
    return JSON.parse(stdout);
  } catch (error) {
    throw new Error(`openhuman-core returned non-JSON output for ${method}: ${stdout}`);
  }
}

function parseArgs(args) {
  const parsed = {
    dryRun: false,
    help: false,
    replace: false,
    workspace: '',
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === '--dry-run') {
      parsed.dryRun = true;
    } else if (arg === '--help' || arg === '-h') {
      parsed.help = true;
    } else if (arg === '--replace') {
      parsed.replace = true;
    } else if (arg === '--workspace') {
      const value = args[index + 1];
      if (!value) throw new Error('--workspace requires a path');
      parsed.workspace = resolve(value);
      index += 1;
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }

  return parsed;
}

function printHelp() {
  console.log(`Seed saved councils through the Rust core registry.

Usage:
  pnpm council:seed [--workspace <path>] [--replace] [--dry-run]

Options:
  --workspace <path>  Use this OPENHUMAN_WORKSPACE root for core persistence.
  --replace           Delete existing councils before seeding.
  --dry-run           Print seed payloads without calling openhuman-core.

Environment:
  OPENHUMAN_CORE_BIN  Override the core binary path. Defaults to target/debug/openhuman-core.
  OPENHUMAN_WORKSPACE Workspace root used by the core when --workspace is omitted.
`);
}
