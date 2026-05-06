#!/usr/bin/env node
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execSync } from 'node:child_process';

function run(cmd, cwd) {
  return execSync(cmd, { cwd, stdio: 'pipe', encoding: 'utf8' });
}

function makeRepo(branchName) {
  const dir = mkdtempSync(path.join(tmpdir(), 'codex-preflight-'));
  mkdirSync(path.join(dir, 'docs/src'), { recursive: true });
  mkdirSync(path.join(dir, 'app'), { recursive: true });
  writeFileSync(path.join(dir, 'AGENTS.md'), '# test\n');
  writeFileSync(path.join(dir, 'docs/src/README.md'), 'ok\n');
  writeFileSync(path.join(dir, 'Cargo.toml'), '[package]\nname="x"\nversion="0.1.0"\n');
  writeFileSync(path.join(dir, 'app/package.json'), '{"name":"x"}\n');
  run('git init', dir);
  run('git config user.email test@example.com', dir);
  run('git config user.name test', dir);
  run('git add .', dir);
  run('git commit -m init', dir);
  run(`git checkout -b ${branchName}`, dir);
  run('git remote add origin git@github.com:jwalin-shah/openhuman.git', dir);
  return dir;
}

const script = path.resolve('scripts/codex-pr-preflight.mjs');
const passRepo = makeRepo('codex/SYM-93-preflight');
run(`CODEX_EXPECT_REPO_PATH=${passRepo} node ${script} --strict-path --lightweight`, passRepo);

const failRepo = makeRepo('feature/not-codex');
let failed = false;
try {
  run(`CODEX_EXPECT_REPO_PATH=${failRepo} node ${script} --strict-path --lightweight`, failRepo);
} catch {
  failed = true;
}
if (!failed) {
  throw new Error('Expected invalid branch naming to fail preflight');
}

console.log('codex preflight self-test passed');
