#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
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

const colors = ['yellow', 'burgundy', 'black', 'navy', 'green'];
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
  const webpFrames = [];

  for (const pngFrame of pngFrames) {
    const webpFrame = pngFrame.replace(/\.png$/u, '.webp');
    run(
      cwebpBinary,
      ['-quiet', '-q', '82', '-m', '6', '-alpha_q', '100', pngFrame, '-o', webpFrame],
      remotionRoot
    );
    webpFrames.push(webpFrame);
  }

  return webpFrames;
}

async function transcodeAnimatedWebp(inputMov, outputWebp, frameDir) {
  extractPngFrames(inputMov, frameDir);
  const webpFrames = await convertPngFramesToWebp(frameDir);
  const frameDurationMs = String(Math.round(1000 / outputFps));
  const args = ['-loop', '0', '-bgcolor', '0,0,0,0'];

  for (const framePath of webpFrames) {
    args.push('-frame', framePath, `+${frameDurationMs}`);
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
