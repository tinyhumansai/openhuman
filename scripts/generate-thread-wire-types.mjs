#!/usr/bin/env node
// Generates UI thread/session record types from Rust-owned JSON Schemas.
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { compile } from 'json-schema-to-typescript';
import { createRequire } from 'node:module';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = resolve(root, 'app/src/types/threadWire.generated.ts');
const prettier = createRequire(resolve(root, 'app/package.json'))('prettier');

function rustSchema() {
  const result = spawnSync(
    'bash',
    [
      resolve(root, 'scripts/ci-cancel-aware.sh'),
      'cargo',
      'run',
      '--quiet',
      '--manifest-path',
      'crates/openhuman-cli/Cargo.toml',
      '-p',
      'openhuman-cli',
      '--example',
      'thread_wire_schema',
    ],
    { cwd: root, encoding: 'utf8' }
  );
  if (result.status !== 0) {
    process.stderr.write(result.stderr || 'failed to export thread wire schema\n');
    process.exit(result.status ?? 1);
  }
  return JSON.parse(result.stdout);
}

async function render() {
  const body = await compile(rustSchema(), 'ThreadWireTypes', {
    bannerComment: '',
    additionalProperties: false,
    unknownAny: false,
    format: false,
    style: { singleQuote: true },
  });
  return prettier.format(
    `/* eslint-disable */\n// @generated from OpenHuman and TinyAgents Rust JSON Schemas.\n// Do not edit by hand; run \`pnpm chat-wire:generate\`.\n\n${body}`,
    { parser: 'typescript', singleQuote: true, printWidth: 100 }
  );
}

const next = await render();
if (process.argv.includes('--check')) {
  const current = readFileSync(output, 'utf8');
  if (current !== next) {
    console.error('threadWire.generated.ts is stale; run `pnpm chat-wire:generate`');
    process.exit(1);
  }
  console.log('threadWire.generated.ts is current');
} else {
  writeFileSync(output, next);
  console.log(`wrote ${output}`);
}
