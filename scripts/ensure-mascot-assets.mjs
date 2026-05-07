#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..');
const generatedRoot = join(repoRoot, 'app', 'public', 'generated', 'remotion');
const remotionRoot = join(repoRoot, 'remotion');
const remotionNodeModules = join(remotionRoot, 'node_modules');
const manifestPath = join(generatedRoot, 'manifest.json');

const colors = ['yellow', 'burgundy', 'black', 'navy', 'green'];
const profiles = ['default', 'compact'];
const compositions = ['yellow-MascotIdle', 'yellow-MascotTalking', 'yellow-MascotThinking'];

function run(command, args, cwd) {
  execFileSync(command, args, {
    cwd,
    stdio: 'inherit',
    env: process.env,
  });
}

function expectedAssetPaths() {
  return profiles.flatMap(profile =>
    colors.flatMap(color =>
      compositions.map(composition => join(generatedRoot, profile, color, `${composition}.webp`))
    )
  );
}

function manifestLooksCurrent() {
  if (!existsSync(manifestPath)) {
    return false;
  }

  try {
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
    const variants = manifest.variants ?? [];
    return (
      variants.length === colors.length * profiles.length * compositions.length &&
      variants.every(variant =>
        colors.includes(variant.color ?? '') &&
        profiles.includes(variant.profile ?? '') &&
        compositions.includes(variant.composition ?? '') &&
        typeof variant.path === 'string' &&
        variant.path.endsWith('.webp')
      )
    );
  } catch {
    return false;
  }
}

function assetsExist() {
  return manifestLooksCurrent() && expectedAssetPaths().every(assetPath => existsSync(assetPath));
}

if (assetsExist()) {
  console.log('[ensure-mascot-assets] mascot asset cache already present');
  process.exit(0);
}

if (!existsSync(remotionNodeModules)) {
  console.log('[ensure-mascot-assets] installing remotion workspace dependencies');
  run('pnpm', ['install', '--frozen-lockfile'], remotionRoot);
}

console.log('[ensure-mascot-assets] generating mascot asset cache');
run('pnpm', ['mascot:render'], repoRoot);
