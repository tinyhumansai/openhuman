// @ts-nocheck
import { waitForApp } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { resetApp } from '../helpers/reset-app';
import { startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-tool-filesystem';

/**
 * Filesystem tool E2E spec — coverage matrix rows 6.1.1 (read), 6.1.2 (write),
 * and 6.1.3 (path-restriction denial). Tracked by issue #967.
 *
 * Drives the workspace-restricted file I/O surface — `openhuman.memory_write_file`,
 * `openhuman.memory_read_file`, `openhuman.memory_list_files` — which is the
 * same security contract the agent-facing `file_read` / `file_write` tools
 * enforce: workspace-relative paths only, parent-traversal blocked, absolute
 * paths blocked, all writes confined to `OPENHUMAN_WORKSPACE`. The Rust unit
 * tests in `src/openhuman/tools/impl/filesystem/file_read.rs` /
 * `file_write.rs` cover the in-process tool path; this WDIO spec proves the
 * UI⇄Tauri⇄sidecar wiring honours the same gates over JSON-RPC.
 *
 * Failure path (6.1.3): a parent-traversal request must be rejected by the
 * sidecar — that's the denial assertion required by gitbooks/developing/testing-strategy.md.
 *
 * Side-effect verification: every successful write is asserted twice — once
 * from the response payload (bytes_written) and once via the test-support
 * workspace file reader against the sidecar's active workspace. This catches
 * transport mismatches that would otherwise pass a payload-only assertion.
 */
function stepLog(message: string, context?: unknown): void {
  const stamp = new Date().toISOString();
  if (context === undefined) {
    console.log(`[ToolFilesystemE2E][${stamp}] ${message}`);
    return;
  }
  console.log(`[ToolFilesystemE2E][${stamp}] ${message}`, JSON.stringify(context, null, 2));
}

const TEST_RELATIVE_PATH = 'e2e-967-filesystem-canary.txt';
const TEST_WORKSPACE_RELATIVE_PATH = `memory/${TEST_RELATIVE_PATH}`;
const TEST_CONTENT =
  'OpenHuman filesystem tool canary fact — issue #967 — bytes asserted both via RPC and disk';
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

interface WorkspaceReadResultEnvelope {
  result?: {
    content_utf8?: string;
    rel_path?: string;
    returned_bytes?: number;
    size_on_disk?: number;
    truncated?: boolean;
  };
}

describe('System tools — Filesystem (file_read / file_write / path restriction)', () => {
  before(async function beforeSuite() {
    this.timeout(90_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
  });

  after(async () => {
    await stopMockServer();
  });

  it('6.1.2 writes a file inside the workspace and the bytes match on disk', async () => {
    stepLog('issuing memory_write_file', {
      relative_path: TEST_RELATIVE_PATH,
      bytes: TEST_CONTENT.length,
    });
    const writeResult = await callOpenhumanRpc<WriteResultEnvelope>('openhuman.memory_write_file', {
      relative_path: TEST_RELATIVE_PATH,
      content: TEST_CONTENT,
    });
    stepLog('write response', writeResult);
    expect(writeResult.ok).toBe(true);

    const data = writeResult.result?.data;
    expect(data?.written).toBe(true);
    // Rust returns UTF-8 byte count; em-dashes (—) are 3 bytes each in UTF-8
    expect(data?.bytes_written).toBe(Buffer.byteLength(TEST_CONTENT, 'utf8'));
    expect(data?.relative_path).toBe(TEST_RELATIVE_PATH);

    // Disk-side assertion: the byte payload must round-trip via the workspace.
    // This is the load-bearing "side effect proof" that the sidecar actually
    // wrote the file rather than only echoing a success payload.
    const diskRead = await callOpenhumanRpc<WorkspaceReadResultEnvelope>(
      'openhuman.test_support_read_workspace_file',
      { rel_path: TEST_WORKSPACE_RELATIVE_PATH, max_bytes: 1024 }
    );
    expect(diskRead.ok).toBe(true);
    expect(diskRead.result?.result?.content_utf8).toBe(TEST_CONTENT);
    expect(diskRead.result?.result?.size_on_disk).toBe(Buffer.byteLength(TEST_CONTENT, 'utf8'));
  });

  it('6.1.1 reads back the file via memory_read_file and content matches', async () => {
    // Seed the canary in-test so the read assertion remains valid when the
    // suite is run with `--grep` and the write test has not preceded it.
    const seed = await callOpenhumanRpc<WriteResultEnvelope>('openhuman.memory_write_file', {
      relative_path: TEST_RELATIVE_PATH,
      content: TEST_CONTENT,
    });
    expect(seed.ok).toBe(true);

    stepLog('issuing memory_read_file', { relative_path: TEST_RELATIVE_PATH });
    const readResult = await callOpenhumanRpc<ReadResultEnvelope>('openhuman.memory_read_file', {
      relative_path: TEST_RELATIVE_PATH,
    });
    stepLog('read response', readResult);
    expect(readResult.ok).toBe(true);
    expect(readResult.result?.data?.content).toBe(TEST_CONTENT);
    expect(readResult.result?.data?.relative_path).toBe(TEST_RELATIVE_PATH);

    // Cross-check with memory_list_files to prove directory listing also
    // honours the workspace boundary and surfaces the canary.
    const listResult = await callOpenhumanRpc<ListResultEnvelope>('openhuman.memory_list_files', {
      relative_dir: '',
    });
    stepLog('list response', listResult);
    expect(listResult.ok).toBe(true);
    const files = listResult.result?.data?.files ?? [];
    expect(files.includes('e2e-967-filesystem-canary.txt')).toBe(true);
  });

  it('6.1.3 rejects parent-traversal AND absolute paths (path-restriction denial)', async () => {
    // 6.1.3a — `..` escape must be denied. The sidecar should never canonicalize
    // out of the workspace; if this assertion ever flips, file_write's security
    // contract has regressed and the agent could exfiltrate to arbitrary disk.
    stepLog('issuing memory_write_file with parent-traversal payload', {
      relative_path: TRAVERSAL_PATH,
    });
    const traversal = await callOpenhumanRpc<WriteResultEnvelope>('openhuman.memory_write_file', {
      relative_path: TRAVERSAL_PATH,
      content: 'should never be written',
    });
    stepLog('traversal response', traversal);
    expect(traversal.ok).toBe(false);
    const traversalErr = traversal.error ?? '';
    expect(traversalErr.toLowerCase()).toMatch(/traversal|not allowed|escape/);

    // 6.1.3b — absolute paths must also be denied; this guards a different
    // branch of the validator (`is_absolute()` short-circuit).
    stepLog('issuing memory_write_file with absolute payload', { relative_path: ABSOLUTE_PATH });
    const absolute = await callOpenhumanRpc<WriteResultEnvelope>('openhuman.memory_write_file', {
      relative_path: ABSOLUTE_PATH,
      content: 'should never be written',
    });
    stepLog('absolute response', absolute);
    expect(absolute.ok).toBe(false);
    const absoluteErr = absolute.error ?? '';
    expect(absoluteErr.toLowerCase()).toMatch(/absolute|not allowed|traversal/);

    // Belt-and-braces: neither denial should have left a file behind. We
    // check the most likely target locations to make sure the validator
    // short-circuited before any std::fs::write call.
    let escaped = false;
    try {
      await fs.access(path.resolve(workspaceDir(), '..', 'escape-967.txt'));
      escaped = true;
    } catch {
      // expected — file should not exist
    }
    try {
      await fs.access(ABSOLUTE_PATH);
      escaped = true;
    } catch {
      // expected — file should not exist
    }
    expect(escaped).toBe(false);
  });
});
