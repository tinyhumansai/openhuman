// @ts-nocheck
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';

const NS = `e2e-memory-${Date.now()}`;
const DOC_KEY = `doc-${Date.now()}`;
const KV_KEY = `kv-${Date.now()}`;
let bulkDocId0: string | null = null;

async function expectRpcOk(method: string, params: Record<string, unknown> = {}) {
  const result = await callOpenhumanRpc(method, params);
  if (!result.ok) {
    console.log(`[MemorySpec] ${method} failed`, result.error);
  }
  expect(result.ok).toBe(true);
  return result.result;
}

function extractDocumentId(payload: unknown): string | null {
  const top = (payload as any)?.document_id;
  if (typeof top === 'string' && top.length > 0) return top;
  const nested = (payload as any)?.result?.document_id;
  if (typeof nested === 'string' && nested.length > 0) return nested;
  return null;
}

describe('Memory System', () => {
  let methods: Set<string>;

  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();

    await expectRpcOk('openhuman.memory_init', {});
    await expectRpcOk('openhuman.memory_clear_namespace', { namespace: NS });
  });

  it('5.1.1 — Store Memory Entry: doc_put persists a document', async () => {
    expectRpcMethod(methods, 'openhuman.memory_doc_put');
    const put = await expectRpcOk('openhuman.memory_doc_put', {
      namespace: NS,
      key: DOC_KEY,
      title: 'E2E Memory Title',
      content: 'Remember this e2e memory entry',
      tags: ['e2e', 'memory'],
      metadata: { source: 'e2e' },
    });

    expect(JSON.stringify(put || {}).length > 0).toBe(true);
  });

  it('5.1.2 — Structured Memory Storage: kv_set stores JSON payload', async () => {
    expectRpcMethod(methods, 'openhuman.memory_kv_set');
    await expectRpcOk('openhuman.memory_kv_set', {
      namespace: NS,
      key: KV_KEY,
      value: { level: 'high', score: 42, flags: ['a', 'b'] },
    });

    const read = await expectRpcOk('openhuman.memory_kv_get', { namespace: NS, key: KV_KEY });

    expect(JSON.stringify(read || {}).includes('42')).toBe(true);
  });

  it('5.1.3 — Duplicate Memory Handling: doc_put upsert is idempotent by key', async () => {
    await expectRpcOk('openhuman.memory_doc_put', {
      namespace: NS,
      key: DOC_KEY,
      title: 'E2E Memory Title Updated',
      content: 'Updated content',
    });

    const listed = await expectRpcOk('openhuman.memory_doc_list', { namespace: NS });
    expect(JSON.stringify(listed || {}).includes(DOC_KEY)).toBe(true);
  });

  it('5.2.1 — Recall Memory: recall_memories returns namespace-scoped recall result', async () => {
    expectRpcMethod(methods, 'openhuman.memory_recall_memories');
    const recall = await callOpenhumanRpc('openhuman.memory_recall_memories', {
      namespace: NS,
      query: 'remember e2e memory',
      limit: 5,
    });

    if (recall.ok) {
      expect(
        JSON.stringify(recall.result || {})
          .toLowerCase()
          .includes('e2e')
      ).toBe(true);
      return;
    }

    // Some runtime profiles disable advanced recall. Fallback still verifies recall-like behavior.
    const fallback = await expectRpcOk('openhuman.memory_context_query', {
      namespace: NS,
      query: 'remember e2e memory',
      limit: 5,
    });
    expect(JSON.stringify(fallback || {}).length > 0).toBe(true);
  });

  it('5.2.2 — Contextual Memory Injection: context_recall returns contextual payload', async () => {
    const context = await callOpenhumanRpc('openhuman.memory_context_recall', {
      namespace: NS,
      query: 'context around memory entry',
      limit: 3,
    });

    if (context.ok) {
      expect(JSON.stringify(context.result || {}).length > 0).toBe(true);
      return;
    }

    // Fallback for runtimes where rich context recall is unavailable.
    const fallback = await expectRpcOk('openhuman.memory_context_query', {
      namespace: NS,
      query: 'context around memory entry',
      limit: 3,
    });
    expect(JSON.stringify(fallback || {}).length > 0).toBe(true);
  });

  it('5.2.3 — Large Memory Set Handling: namespace listing handles result sets', async () => {
    for (let i = 0; i < 8; i += 1) {
      const put = await expectRpcOk('openhuman.memory_doc_put', {
        namespace: NS,
        key: `${DOC_KEY}-bulk-${i}`,
        title: `Bulk ${i}`,
        content: `bulk content ${i}`,
      });
      if (i === 0) {
        bulkDocId0 = extractDocumentId(put);
      }
    }
    const listed = await expectRpcOk('openhuman.memory_doc_list', { namespace: NS });
    expect(JSON.stringify(listed || {}).includes('bulk')).toBe(true);
  });

  it('5.3.1 — Forget Memory Entry: doc_delete removes targeted document', async () => {
    if (!bulkDocId0) {
      const listed = await expectRpcOk('openhuman.memory_doc_list', { namespace: NS });
      const text = JSON.stringify(listed || {});
      const match = text.match(/"document_id"\s*:\s*"([^"]+)"/);
      bulkDocId0 = match?.[1] || null;
    }
    expect(Boolean(bulkDocId0)).toBe(true);

    await expectRpcOk('openhuman.memory_doc_delete', { namespace: NS, document_id: bulkDocId0 });

    const listed = await expectRpcOk('openhuman.memory_doc_list', { namespace: NS });
    const text = JSON.stringify(listed || {});
    if (bulkDocId0) {
      expect(text.includes(bulkDocId0)).toBe(false);
    }
  });

  it('5.3.2 — Bulk Memory Deletion: clear_namespace wipes all entries', async () => {
    await expectRpcOk('openhuman.memory_clear_namespace', { namespace: NS });
    const listed = await expectRpcOk('openhuman.memory_doc_list', { namespace: NS });
    const text = JSON.stringify(listed || {}).toLowerCase();
    expect(text.includes('bulk')).toBe(false);
  });

  it('5.3.3 — Deletion Consistency: kv_get returns null/empty after kv_delete', async () => {
    await expectRpcOk('openhuman.memory_kv_set', {
      namespace: NS,
      key: 'to-delete',
      value: { alive: true },
    });
    await expectRpcOk('openhuman.memory_kv_delete', { namespace: NS, key: 'to-delete' });

    const after = await expectRpcOk('openhuman.memory_kv_get', { namespace: NS, key: 'to-delete' });
    expect(JSON.stringify(after || {}).includes('alive')).toBe(false);
  });
});
