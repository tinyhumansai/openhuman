#!/usr/bin/env node

import { execFile, execFileSync } from 'node:child_process';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);
import { chmodSync, existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import os from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const __dirname = dirname(fileURLToPath(import.meta.url));
const remotionRoot = resolve(__dirname, '..');
const outputRoot = resolve(remotionRoot, '..', 'app', 'public', 'generated', 'remotion');
const tempRoot = resolve(remotionRoot, 'out', 'runtime-assets');
const remotionBinary = join(remotionRoot, 'node_modules', '.bin', 'remotion');
const outputFps = 30;
const webpPackageRoot = dirname(require.resolve('webp-converter/package.json'));

function resolveWebpBinary(name) {
  switch (os.platform()) {
    case 'darwin':
      return join(webpPackageRoot, 'bin', 'libwebp_osx', 'bin', name);
    case 'linux':
      return join(webpPackageRoot, 'bin', 'libwebp_linux', 'bin', name);
    case 'win32':
      return join(webpPackageRoot, 'bin', 'libwebp_win64', 'bin', `${name}.exe`);
    default:
      throw new Error(`[remotion-runtime-assets] unsupported platform for vendored webp tools: ${os.platform()}`);
  }
}

const cwebpBinary = resolveWebpBinary('cwebp');
const webpmuxBinary = resolveWebpBinary('webpmux');

const ALL_COLORS = ['yellow', 'burgundy', 'black', 'navy', 'green'];

function resolveColorSet() {
  const raw = (process.env.MASCOT_COLORS ?? '').trim();
  if (!raw) return ['yellow'];
  if (raw.toLowerCase() === 'all') return ALL_COLORS;
  const requested = raw
    .split(',')
    .map(s => s.trim())
    .filter(Boolean);
  const unknown = requested.filter(c => !ALL_COLORS.includes(c));
  if (unknown.length > 0) {
    throw new Error(
      `[remotion-runtime-assets] unknown mascot color(s): ${unknown.join(', ')}. Allowed: ${ALL_COLORS.join(', ')}, or "all".`
    );
  }
  if (!requested.includes('yellow')) {
    requested.unshift('yellow');
  }
  return requested;
}

const colors = resolveColorSet();
console.log(`[remotion-runtime-assets] rendering colors: ${colors.join(', ')}`);
const baseVariants = [
  { composition: 'yellow-MascotIdle', profile: 'default', props: {} },
  { composition: 'yellow-MascotTalking', profile: 'default', props: {} },
  { composition: 'yellow-MascotThinking', profile: 'default', props: {} },
  {
    composition: 'yellow-MascotIdle',
    profile: 'compact',
    props: { groundShadowOpacity: 0.75, compactArmShading: true },
  },
  {
    composition: 'yellow-MascotTalking',
    profile: 'compact',
    props: { groundShadowOpacity: 0.75, compactArmShading: true },
  },
  {
    composition: 'yellow-MascotThinking',
    profile: 'compact',
    props: { groundShadowOpacity: 0.75, compactArmShading: true },
  },
];
const variants = baseVariants.flatMap(variant =>
  colors.map(color => ({
    ...variant,
    color,
    props: { ...variant.props, mascotColor: color },
  }))
);

function run(command, args, cwd) {
  try {
    execFileSync(command, args, {
      cwd,
      stdio: 'inherit',
      env: process.env,
    });
  } catch (error) {
    if (error?.code === 'ENOENT') {
      throw new Error(
        `[remotion-runtime-assets] missing required executable "${command}". Install it and ensure it is on PATH.`
      );
    }
    throw error;
  }
}

function ensureCleanDir(dir) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
}

function ensureExecutable(path) {
  if (os.platform() !== 'win32') {
    chmodSync(path, 0o755);
  }
}

function renderMov(composition, destination, props) {
  if (!existsSync(remotionBinary)) {
    throw new Error(`remotion CLI missing at ${remotionBinary}; run pnpm install in remotion/ first`);
  }

  const args = [
    'render',
    composition,
    destination,
    '--codec=prores',
    '--prores-profile=4444',
    '--pixel-format=yuva444p10le',
  ];
  if (Object.keys(props).length > 0) {
    args.push('--props', JSON.stringify(props));
  }
  run(remotionBinary, args, remotionRoot);
}

function extractPngFrames(inputMov, frameDir) {
  mkdirSync(frameDir, { recursive: true });
  run(
    'ffmpeg',
    [
      '-y',
      '-i',
      inputMov,
      '-an',
      '-vsync',
      'passthrough',
      '-pix_fmt',
      'rgba',
      '-start_number',
      '0',
      join(frameDir, 'frame-%04d.png'),
    ],
    remotionRoot
  );
}

function listFrames(frameDir, extension) {
  return readdirSync(frameDir)
    .filter(entry => entry.endsWith(extension))
    .sort()
    .map(entry => join(frameDir, entry));
}

async function convertPngFramesToWebp(frameDir) {
  const pngFrames = listFrames(frameDir, '.png');
  const webpFrames = new Array(pngFrames.length);
  const concurrency = Math.max(1, Math.min(os.cpus()?.length ?? 4, 8));
  let nextIndex = 0;
  let completed = 0;
  const total = pngFrames.length;

  async function worker() {
    while (true) {
      const idx = nextIndex++;
      if (idx >= total) return;
      const pngFrame = pngFrames[idx];
      const webpFrame = pngFrame.replace(/\.png$/u, '.webp');
      try {
        await execFileAsync(
          cwebpBinary,
          ['-quiet', '-q', '82', '-m', '4', '-alpha_q', '100', pngFrame, '-o', webpFrame],
          { cwd: remotionRoot }
        );
      } catch (error) {
        if (error?.code === 'ENOENT') {
          throw new Error(
            `[remotion-runtime-assets] missing required executable "${cwebpBinary}". Install it and ensure it is on PATH.`
          );
        }
        throw error;
      }
      webpFrames[idx] = webpFrame;
      completed += 1;
      if (completed === total || completed % 30 === 0) {
        console.log(`[remotion-runtime-assets]   cwebp ${completed}/${total} frames`);
      }
    }
  }

  await Promise.all(Array.from({ length: concurrency }, () => worker()));
  return webpFrames;
}

async function transcodeAnimatedWebp(inputMov, outputWebp, frameDir) {
  extractPngFrames(inputMov, frameDir);
  const webpFrames = await convertPngFramesToWebp(frameDir);
  const frameDurationMs = String(Math.round(1000 / outputFps));
  const args = ['-loop', '0', '-bgcolor', '0,0,0,0'];

  // webpmux frame options: +duration+xoff+yoff+dispose+blend
  //   dispose=1 → clear canvas to background (transparent) before drawing the
  //     next frame. Without this, frames composite over previous ones and
  //     transparent mascot poses ghost on top of each other.
  //   -b → no blending; the frame's RGBA replaces the canvas pixels. With
  //     blending the alpha of the prior frame leaks through even after a
  //     dispose, producing a faint overlay around the silhouette.
  for (const framePath of webpFrames) {
    args.push('-frame', framePath, `+${frameDurationMs}+0+0+1-b`);
  }

  args.push('-o', outputWebp);
  run(webpmuxBinary, args, remotionRoot);
}

ensureCleanDir(outputRoot);
ensureCleanDir(tempRoot);
ensureExecutable(cwebpBinary);
ensureExecutable(webpmuxBinary);

const manifest = {
  generatedAt: new Date().toISOString(),
  format: 'image/webp',
  variants: [],
};

for (const variant of variants) {
  const profileDir = join(outputRoot, variant.profile, variant.color);
  const tempVariantDir = join(tempRoot, variant.profile, variant.color, variant.composition);
  mkdirSync(profileDir, { recursive: true });
  mkdirSync(tempVariantDir, { recursive: true });

  const movPath = join(tempVariantDir, `${variant.composition}.mov`);
  const webpPath = join(profileDir, `${variant.composition}.webp`);
  const frameDir = join(tempVariantDir, 'frames');

  console.log(
    `[remotion-runtime-assets] rendering ${variant.profile}/${variant.color}/${variant.composition}`
  );
  renderMov(variant.composition, movPath, variant.props);
  await transcodeAnimatedWebp(movPath, webpPath, frameDir);

  manifest.variants.push({
    color: variant.color,
    composition: variant.composition,
    profile: variant.profile,
    path: `${variant.profile}/${variant.color}/${variant.composition}.webp`,
    props: variant.props,
  });
}

writeFileSync(join(outputRoot, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
rmSync(tempRoot, { recursive: true, force: true });

console.log(`[remotion-runtime-assets] wrote assets to ${outputRoot}`);
