import { expect, test } from '@playwright/test';
import { promises as fs } from 'node:fs';
import path from 'node:path';

import { bootAuthenticatedPage, callCoreRpc } from '../helpers/core-rpc';

const TEST_RELATIVE_PATH = 'e2e-967-filesystem-canary.txt';
const TEST_CONTENT =
  'OpenHuman filesystem tool canary fact - issue #967 - bytes asserted both via RPC and disk';
const TRAVERSAL_PATH = '../escape-967.txt';
const ABSOLUTE_PATH = '/tmp/openhuman-967-absolute-escape.txt';

interface WriteResultEnvelope {
  data?: { relative_path?: string; written?: boolean; bytes_written?: number };
}

interface ReadResultEnvelope {
  data?: { relative_path?: string; content?: string };
}

interface ListResultEnvelope {
  data?: { relative_dir?: string; files?: string[]; count?: number };
}

function workspaceDir(): string {
  const ws = process.env.OPENHUMAN_WORKSPACE;
  if (!ws) {
    throw new Error('OPENHUMAN_WORKSPACE not set for tool-filesystem-flow Playwright run');
  }
  return ws;
}

test.describe('System tools - Filesystem', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const testSlug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, 'pw-tool-filesystem-' + testSlug, '/home');
  });

  test('writes a file inside the workspace and bytes match on disk', async () => {
    const writeResult = await callCoreRpc<WriteResultEnvelope>('openhuman.memory_write_file', {
      relative_path: TEST_RELATIVE_PATH,
      content: TEST_CONTENT,
    });
    const data = writeResult.data;
    expect(data?.written).toBe(true);
    expect(data?.bytes_written).toBe(Buffer.byteLength(TEST_CONTENT, 'utf8'));
    expect(data?.relative_path).toBe(TEST_RELATIVE_PATH);

    const diskPath = path.join(
      workspaceDir(),
      'workspace',
      'memory',
      data?.relative_path ?? TEST_RELATIVE_PATH
    );
    const diskContents = await fs.readFile(diskPath, 'utf8');
    const diskStat = await fs.stat(diskPath);
    expect(diskContents).toBe(TEST_CONTENT);
    expect(diskStat.size).toBe(Buffer.byteLength(TEST_CONTENT, 'utf8'));
  });

  test('reads back the file and list_files surfaces it', async () => {
    await callCoreRpc<WriteResultEnvelope>('openhuman.memory_write_file', {
      relative_path: TEST_RELATIVE_PATH,
      content: TEST_CONTENT,
    });

    const readResult = await callCoreRpc<ReadResultEnvelope>('openhuman.memory_read_file', {
      relative_path: TEST_RELATIVE_PATH,
    });
    expect(readResult.data?.content).toBe(TEST_CONTENT);
    expect(readResult.data?.relative_path).toBe(TEST_RELATIVE_PATH);

    const listResult = await callCoreRpc<ListResultEnvelope>('openhuman.memory_list_files', {
      relative_dir: '',
    });
    const files = listResult.data?.files ?? [];
    expect(files.includes('e2e-967-filesystem-canary.txt')).toBe(true);
  });

  test('rejects parent-traversal and absolute paths', async () => {
    await expect(
      callCoreRpc<WriteResultEnvelope>('openhuman.memory_write_file', {
        relative_path: TRAVERSAL_PATH,
        content: 'should never be written',
      })
    ).rejects.toThrow(/traversal|not allowed|escape/i);

    await expect(
      callCoreRpc<WriteResultEnvelope>('openhuman.memory_write_file', {
        relative_path: ABSOLUTE_PATH,
        content: 'should never be written',
      })
    ).rejects.toThrow(/absolute|not allowed|traversal/i);

    let escaped = false;
    try {
      await fs.access(path.resolve(workspaceDir(), '..', 'escape-967.txt'));
      escaped = true;
    } catch {}
    try {
      await fs.access(ABSOLUTE_PATH);
      escaped = true;
    } catch {}
    expect(escaped).toBe(false);
  });
});
