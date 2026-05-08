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

const ALL_COLORS = ['yellow', 'burgundy', 'black', 'navy', 'green'];
const profiles = ['default', 'compact'];
const compositions = ['yellow-MascotIdle', 'yellow-MascotTalking', 'yellow-MascotThinking'];

function resolveColorSet() {
  const raw = (process.env.MASCOT_COLORS ?? '').trim();
  if (!raw) return ['yellow'];
  if (raw.toLowerCase() === 'all') return ALL_COLORS;
  const requested = raw
    .split(',')
    .map(s => s.trim())
    .filter(Boolean)
    .filter(c => ALL_COLORS.includes(c));
  if (!requested.includes('yellow')) {
    requested.unshift('yellow');
  }
  return requested.length > 0 ? requested : ['yellow'];
}

const colors = resolveColorSet();

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
    // Allow caches that include MORE colors than requested (e.g. CI cache restored locally)
    const presentTuples = new Set(
      variants
        .filter(variant =>
          ALL_COLORS.includes(variant.color ?? '') &&
          profiles.includes(variant.profile ?? '') &&
          compositions.includes(variant.composition ?? '') &&
          typeof variant.path === 'string' &&
          variant.path.endsWith('.webp')
        )
        .map(variant => `${variant.profile}/${variant.color}/${variant.composition}`)
    );
    for (const profile of profiles) {
      for (const color of colors) {
        for (const composition of compositions) {
          if (!presentTuples.has(`${profile}/${color}/${composition}`)) {
            return false;
          }
        }
      }
    }
    return true;
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
  run('pnpm', ['install', '--ignore-workspace', '--frozen-lockfile'], remotionRoot);
}

console.log('[ensure-mascot-assets] generating mascot asset cache');
run('pnpm', ['mascot:render'], repoRoot);
