#!/usr/bin/env node
'use strict';

// postinstall: downloads the correct pre-built binary for this platform/arch,
// verifies the SHA-256 checksum, then places it at bin/openhuman-bin[.exe].
//
// The binary is fetched from the GitHub release that matches package.json version.

const https = require('https');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { execFileSync } = require('child_process');

const REPO = 'tinyhumansai/openhuman';
const pkg = require('./package.json');
const VERSION = pkg.version;

// Maps process.platform + process.arch → Rust target triple
const TARGET_MAP = {
  darwin: { x64: 'x86_64-apple-darwin', arm64: 'aarch64-apple-darwin' },
  linux: { x64: 'x86_64-unknown-linux-gnu', arm64: 'aarch64-unknown-linux-gnu' },
  win32: { x64: 'x86_64-pc-windows-msvc' },
};

function getTarget() {
  const platform = process.platform;
  const arch = process.arch;
  const targets = TARGET_MAP[platform];
  if (!targets) throw new Error(`Unsupported platform: ${platform}`);
  const target = targets[arch];
  if (!target) throw new Error(`Unsupported arch ${arch} on ${platform}`);
  return { platform, target };
}

function httpsGet(url) {
  return new Promise((resolve, reject) => {
    function request(u) {
      https.get(u, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          return request(res.headers.location);
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error(`HTTP ${res.statusCode} fetching ${u}`));
        }
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => resolve(Buffer.concat(chunks)));
        res.on('error', reject);
      }).on('error', reject);
    }
    request(url);
  });
}

function downloadFile(url, dest) {
  return new Promise((resolve, reject) => {
    function request(u) {
      https.get(u, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          return request(res.headers.location);
        }
        if (res.statusCode !== 200) {
          res.resume();
          return reject(new Error(`HTTP ${res.statusCode} fetching ${u}`));
        }
        const out = fs.createWriteStream(dest);
        res.pipe(out);
        out.on('finish', () => out.close(resolve));
        out.on('error', reject);
        res.on('error', reject);
      }).on('error', reject);
    }
    request(url);
  });
}

function sha256hex(filePath) {
  return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
}

async function main() {
  // Skip in CI environments that just need the package metadata
  if (process.env.SKIP_OPENHUMAN_BINARY_DOWNLOAD) {
    console.log('[openhuman] Skipping binary download (SKIP_OPENHUMAN_BINARY_DOWNLOAD set)');
    return;
  }

  const { platform, target } = getTarget();
  const isWin = platform === 'win32';
  const ext = isWin ? '.zip' : '.tar.gz';
  const tarball = `openhuman-core-${VERSION}-${target}${ext}`;
  const checksumFile = `${tarball}.sha256`;
  const baseUrl = `https://github.com/${REPO}/releases/download/v${VERSION}`;

  const binDir = path.join(__dirname, 'bin');
  fs.mkdirSync(binDir, { recursive: true });

  const tmpTarball = path.join(binDir, tarball);
  const binDest = path.join(binDir, isWin ? 'openhuman-bin.exe' : 'openhuman-bin');

  // Skip if binary already exists and is executable
  if (fs.existsSync(binDest)) {
    console.log('[openhuman] Binary already installed, skipping download.');
    return;
  }

  console.log(`[openhuman] Downloading v${VERSION} for ${target}...`);

  // Download checksum first (small)
  const checksumData = await httpsGet(`${baseUrl}/${checksumFile}`);
  const expectedChecksum = checksumData.toString('utf8').trim().split(/\s+/)[0];

  // Download binary archive
  await downloadFile(`${baseUrl}/${tarball}`, tmpTarball);

  // Verify checksum
  const actualChecksum = sha256hex(tmpTarball);
  if (expectedChecksum !== actualChecksum) {
    fs.rmSync(tmpTarball, { force: true });
    throw new Error(
      `[openhuman] Checksum mismatch!\n  expected: ${expectedChecksum}\n  got:      ${actualChecksum}`
    );
  }
  console.log('[openhuman] Checksum verified.');

  // Extract — use execFileSync (no shell interpolation) so paths with spaces
  // or shell metacharacters in `tmpTarball` / `binDir` can't be injected.
  if (isWin) {
    // PowerShell is available on Windows runners
    execFileSync(
      'powershell',
      [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        `Expand-Archive -Path $env:TC_SRC -DestinationPath $env:TC_DEST -Force`,
      ],
      { stdio: 'inherit', env: { ...process.env, TC_SRC: tmpTarball, TC_DEST: binDir } }
    );
    const extracted = path.join(binDir, 'openhuman-core.exe');
    if (fs.existsSync(extracted)) fs.renameSync(extracted, binDest);
  } else {
    execFileSync('tar', ['-xzf', tmpTarball, '-C', binDir], { stdio: 'inherit' });
    const extracted = path.join(binDir, 'openhuman-core');
    if (fs.existsSync(extracted)) {
      fs.renameSync(extracted, binDest);
      fs.chmodSync(binDest, 0o755);
    }
  }

  // Clean up archive
  fs.rmSync(tmpTarball, { force: true });

  if (!fs.existsSync(binDest)) {
    throw new Error('[openhuman] Extraction failed — binary not found after unpack.');
  }

  console.log(`[openhuman] Installed at ${binDest}`);
}

main().catch((err) => {
  console.error('\n[openhuman] Installation failed:', err.message);
  console.error('You can file a bug at https://github.com/tinyhumansai/openhuman/issues');
  process.exit(1);
});
