// @ts-nocheck
/**
 * Agent-observable artifact capture for E2E specs.
 *
 * Creates a per-run directory under app/test/e2e/artifacts/ and provides
 * helpers to drop screenshots, page-source dumps, mock request-log snapshots,
 * and a meta.json that agents (and humans) can inspect from disk.
 *
 * Layout:
 *   app/test/e2e/artifacts/
 *     2026-04-21T23-15-10Z-agent-review/
 *       01-welcome.png
 *       01-welcome.source.xml
 *       02-privacy-sheet.png
 *       02-privacy-sheet.source.xml
 *       failure-<test>.png
 *       failure-<test>.source.xml
 *       mock-requests-<checkpoint>.json
 *       meta.json
 *
 * Env:
 *   E2E_ARTIFACT_DIR — overrides the auto-generated run dir.
 *   E2E_ARTIFACT_ROOT — overrides the artifacts/ parent dir.
 */
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

import { dumpAccessibilityTree } from './element-helpers';

const thisDir = path.dirname(fileURLToPath(import.meta.url));
const defaultRoot = path.resolve(thisDir, '..', 'artifacts');

type Meta = {
  runId: string;
  startedAt: string;
  platform: NodeJS.Platform;
  checkpoints: { index: number; name: string; at: string; files: string[] }[];
  failures: { testName: string; at: string; files: string[] }[];
};

let runDir: string | null = null;
let meta: Meta | null = null;
let checkpointIndex = 0;

function sanitize(name: string): string {
  return name.replace(/[^a-zA-Z0-9_.-]+/g, '-').slice(0, 80) || 'unnamed';
}

function nowStamp(): string {
  return new Date().toISOString().replace(/[:.]/g, '-').replace('Z', 'Z');
}

function getRoot(): string {
  return process.env.E2E_ARTIFACT_ROOT ? path.resolve(process.env.E2E_ARTIFACT_ROOT) : defaultRoot;
}

/**
 * Compute + create the per-run artifact directory. Idempotent.
 * Returns the absolute path.
 */
export function getArtifactDir(): string {
  if (runDir) return runDir;

  if (process.env.E2E_ARTIFACT_DIR) {
    runDir = path.resolve(process.env.E2E_ARTIFACT_DIR);
  } else {
    const label = sanitize(process.env.E2E_ARTIFACT_LABEL || 'run');
    runDir = path.join(getRoot(), `${nowStamp()}-${label}`);
  }

  fs.mkdirSync(runDir, { recursive: true });

  meta = {
    runId: path.basename(runDir),
    startedAt: new Date().toISOString(),
    platform: process.platform,
    checkpoints: [],
    failures: [],
  };
  writeMeta();

  // eslint-disable-next-line no-console
  console.log(`[artifacts] run dir: ${runDir}`);
  return runDir;
}

function writeMeta(): void {
  if (!runDir || !meta) return;
  fs.writeFileSync(path.join(runDir, 'meta.json'), JSON.stringify(meta, null, 2));
}

async function writeScreenshot(file: string): Promise<boolean> {
  try {
    const png = await browser.takeScreenshot();
    fs.writeFileSync(file, Buffer.from(png, 'base64'));
    return true;
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn(`[artifacts] screenshot failed: ${err}`);
    return false;
  }
}

async function writeSource(file: string): Promise<boolean> {
  try {
    const source = await dumpAccessibilityTree();
    fs.writeFileSync(file, source);
    return true;
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn(`[artifacts] source dump failed: ${err}`);
    return false;
  }
}

/**
 * Capture a named checkpoint: screenshot + page source.
 * Numbered so agents can read the flow chronologically.
 */
export async function captureCheckpoint(name: string): Promise<void> {
  const dir = getArtifactDir();
  checkpointIndex += 1;
  const idx = String(checkpointIndex).padStart(2, '0');
  const base = `${idx}-${sanitize(name)}`;

  const pngFile = path.join(dir, `${base}.png`);
  const xmlFile = path.join(dir, `${base}.source.xml`);

  const files: string[] = [];
  if (await writeScreenshot(pngFile)) files.push(path.basename(pngFile));
  if (await writeSource(xmlFile)) files.push(path.basename(xmlFile));

  if (meta) {
    meta.checkpoints.push({ index: checkpointIndex, name, at: new Date().toISOString(), files });
    writeMeta();
  }
  // eslint-disable-next-line no-console
  console.log(`[artifacts] checkpoint ${idx} "${name}" → ${files.join(', ')}`);
}

/**
 * Always-on failure hook: screenshot + source named after the failing test.
 * Safe to call from wdio afterTest without crashing the runner.
 */
export async function captureFailureArtifacts(testName: string): Promise<void> {
  try {
    const dir = getArtifactDir();
    const base = `failure-${sanitize(testName)}`;
    const pngFile = path.join(dir, `${base}.png`);
    const xmlFile = path.join(dir, `${base}.source.xml`);

    const files: string[] = [];
    if (await writeScreenshot(pngFile)) files.push(path.basename(pngFile));
    if (await writeSource(xmlFile)) files.push(path.basename(xmlFile));

    if (meta) {
      meta.failures.push({ testName, at: new Date().toISOString(), files });
      writeMeta();
    }
    // eslint-disable-next-line no-console
    console.log(`[artifacts] FAILURE "${testName}" → ${files.join(', ')}`);
  } catch (err) {
    // Never let artifact capture break the runner.
    // eslint-disable-next-line no-console
    console.warn(`[artifacts] captureFailureArtifacts swallow: ${err}`);
  }
}

/**
 * Persist the current mock-server request log next to the checkpoints.
 * Accepts the log array from getRequestLog() to avoid coupling to mock-server here.
 */
export function saveMockRequestLog(label: string, log: unknown[]): string {
  const dir = getArtifactDir();
  const file = path.join(dir, `mock-requests-${sanitize(label)}.json`);
  fs.writeFileSync(file, JSON.stringify(log, null, 2));
  // eslint-disable-next-line no-console
  console.log(`[artifacts] mock log "${label}" (${log.length} req) → ${path.basename(file)}`);
  return file;
}

/**
 * Reset helper for tests that create multiple runs in one process.
 */
export function resetArtifactRun(): void {
  runDir = null;
  meta = null;
  checkpointIndex = 0;
}
