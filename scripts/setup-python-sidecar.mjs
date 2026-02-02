#!/usr/bin/env node
/**
 * Symlink system python3 to src-tauri/runtime-skill-python-<target> for local dev.
 * Run from project root: node scripts/setup-python-sidecar.mjs
 */
import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, '..');
const binariesDir = path.join(root, 'src-tauri');

function getTargetTriple() {
  try {
    return execSync('rustc --print host-tuple', { encoding: 'utf8' }).trim();
  } catch {
    const out = execSync('rustc -Vv', { encoding: 'utf8' });
    const m = out.match(/host:\s*(.+)/);
    if (m) return m[1].trim();
    throw new Error('Could not get Rust target triple (rustc not found?)');
  }
}

function getSystemPython() {
  const isWin = process.platform === 'win32';
  try {
    const cmd = isWin ? 'where python' : 'which python3';
    const out = execSync(cmd, { encoding: 'utf8' }).trim();
    const first = out.split(/[\r\n]/)[0].trim();
    if (first) return first;
  } catch {}
  throw new Error(
    'System Python not found. Install Python 3 and ensure python3 (or python on Windows) is on PATH.'
  );
}

const target = getTargetTriple();
const ext = process.platform === 'win32' ? '.exe' : '';
const sidecarName = `runtime-skill-python-${target}${ext}`;
const sidecarPath = path.join(binariesDir, sidecarName);
const systemPython = getSystemPython();

try {
  if (fs.existsSync(sidecarPath)) {
    fs.unlinkSync(sidecarPath);
  }
  fs.symlinkSync(systemPython, sidecarPath);
  console.log(`Linked ${sidecarName} -> ${systemPython}`);
} catch (err) {
  console.error(err.message);
  process.exit(1);
}
