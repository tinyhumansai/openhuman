#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const remotionRoot = resolve(__dirname, '..');
const outputRoot = resolve(remotionRoot, '..', 'app', 'public', 'generated', 'remotion');
const tempRoot = resolve(remotionRoot, 'out', 'runtime-assets');
const remotionBinary = join(remotionRoot, 'node_modules', '.bin', 'remotion');

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
  execFileSync(command, args, {
    cwd,
    stdio: 'inherit',
    env: process.env,
  });
}

function ensureCleanDir(dir) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
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

function transcodeWebp(inputMov, outputWebp) {
  run(
    'ffmpeg',
    [
      '-y',
      '-i',
      inputMov,
      '-an',
      '-vcodec',
      'libwebp',
      '-quality',
      '82',
      '-compression_level',
      '6',
      '-loop',
      '0',
      '-pix_fmt',
      'yuva420p',
      outputWebp,
    ],
    remotionRoot
  );
}

ensureCleanDir(outputRoot);
ensureCleanDir(tempRoot);

const manifest = {
  generatedAt: new Date().toISOString(),
  format: 'image/webp',
  variants: [],
};

for (const variant of variants) {
  const profileDir = join(outputRoot, variant.profile, variant.color);
  const tempProfileDir = join(tempRoot, variant.profile);
  mkdirSync(profileDir, { recursive: true });
  mkdirSync(tempProfileDir, { recursive: true });

  const movPath = join(tempProfileDir, `${variant.composition}.mov`);
  const webpPath = join(profileDir, `${variant.composition}.webp`);

  console.log(
    `[remotion-runtime-assets] rendering ${variant.profile}/${variant.color}/${variant.composition}`
  );
  renderMov(variant.composition, movPath, variant.props);
  transcodeWebp(movPath, webpPath);

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
