#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { execSync } from 'node:child_process';

const REQUIRED_FILES = ['AGENTS.md', 'docs/src/README.md', 'Cargo.toml', 'app/package.json'];
const APP_PATTERNS = [/^app\//, /^docs\//];
const ROOT_RUST_PATTERNS = [/^src\//, /^tests\//, /^Cargo\.toml$/, /^Cargo\.lock$/];
const TAURI_PATTERNS = [/^app\/src-tauri\//];

function hasPattern(files, patterns) {
  return files.some((file) => patterns.some((pattern) => pattern.test(file)));
}

function runGit(command, repoRoot) {
  return execSync(command, { cwd: repoRoot, encoding: 'utf8' }).trim();
}

function parseArgs(argv) {
  return {
    lightweight: argv.includes('--lightweight'),
    strictPath: argv.includes('--strict-path'),
    expectedPath: process.env.CODEX_EXPECT_REPO_PATH || '/workspace/openhuman',
  };
}

function runCheck(label, ok, details = '') {
  return { label, ok, details };
}

function summarize(checks) {
  const failed = checks.filter((check) => !check.ok);
  for (const check of checks) {
    const prefix = check.ok ? 'PASS' : 'FAIL';
    const details = check.details ? ` :: ${check.details}` : '';
    console.log(`[${prefix}] ${check.label}${details}`);
  }
  return failed.length;
}

function recommendations(changedFiles, lightweight) {
  const lines = [];
  if (hasPattern(changedFiles, APP_PATTERNS)) {
    lines.push('pnpm --filter openhuman-app format:check');
    lines.push('pnpm typecheck');
    lines.push('pnpm --dir app exec vitest run <changed-test-files> --config test/vitest.config.ts');
  }
  if (hasPattern(changedFiles, ROOT_RUST_PATTERNS)) {
    lines.push('cargo fmt --manifest-path Cargo.toml --all --check');
    if (!lightweight) lines.push('pnpm debug rust <test-filter>');
  }
  if (hasPattern(changedFiles, TAURI_PATTERNS)) {
    lines.push('cargo fmt --manifest-path app/src-tauri/Cargo.toml --all --check');
  }
  return [...new Set(lines)];
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const repoRoot = process.cwd();
  const checks = [];

  checks.push(runCheck('working directory exists', fs.existsSync(repoRoot), repoRoot));
  if (options.strictPath) {
    checks.push(runCheck('expected repo path', path.resolve(repoRoot) === path.resolve(options.expectedPath), `expected ${options.expectedPath}, got ${repoRoot}`));
  }

  for (const file of REQUIRED_FILES) {
    checks.push(runCheck(`required file: ${file}`, fs.existsSync(path.join(repoRoot, file))));
  }

  let branch = '';
  let remotes = '';
  let changed = '';
  try {
    branch = runGit('git branch --show-current', repoRoot);
    checks.push(runCheck('branch naming convention', /^codex\/[A-Z]+-\d+[-a-z0-9]*$/i.test(branch), branch));
  } catch (error) {
    checks.push(runCheck('branch readable', false, String(error)));
  }

  try {
    remotes = runGit('git remote -v', repoRoot);
    if (!remotes) {
      checks.push(runCheck('git remote configured (recommended)', true, 'no remotes configured in this checkout'));
    } else {
      const hasExpectedRemote = /(jwalin-shah\/openhuman|tinyhumansai\/openhuman)/.test(remotes);
      checks.push(runCheck('expected git remote present', hasExpectedRemote));
    }
  } catch (error) {
    checks.push(runCheck('git remote readable', false, String(error)));
  }

  try {
    changed = runGit('git diff --name-only --diff-filter=ACMR HEAD', repoRoot);
    checks.push(runCheck('changed files readable', true, changed || 'no changed files'));
  } catch (error) {
    checks.push(runCheck('changed files readable', false, String(error)));
  }

  const changedFiles = changed ? changed.split('\n').filter(Boolean) : [];
  const commandRecommendations = recommendations(changedFiles, options.lightweight);

  console.log('\nRecommended validation commands:');
  if (commandRecommendations.length === 0) {
    console.log('- (none) no changed files detected');
  } else {
    for (const cmd of commandRecommendations) console.log(`- ${cmd}`);
  }

  const failures = summarize(checks);
  if (failures > 0) process.exit(1);
}

main();
