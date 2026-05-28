import { expect, test } from '@playwright/test';

import { bootAuthenticatedPage, callCoreRpc } from '../helpers/core-rpc';

const TEST_NAMESPACE = 'e2e-memory-roundtrip-773';
const TEST_KEY = 'roundtrip-canary-key';
const TEST_TITLE = 'Memory roundtrip canary';
const TEST_CONTENT = 'OpenHuman memory roundtrip canary fact #773';

test.describe('Memory subsystem round-trip', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    const slug = testInfo.title.toLowerCase().replace(/[^a-z0-9]+/g, '-');
    await bootAuthenticatedPage(page, `pw-memory-roundtrip-${slug}`, '/home');

    await callCoreRpc<unknown>('openhuman.memory_init', { jwt_token: '' });
    await callCoreRpc<unknown>('openhuman.memory_clear_namespace', { namespace: TEST_NAMESPACE });
  });

  test('stores a document and finds it via recall_memories', async () => {
    const storeResult = await callCoreRpc<unknown>('openhuman.memory_doc_put', {
      namespace: TEST_NAMESPACE,
      key: TEST_KEY,
      title: TEST_TITLE,
      content: TEST_CONTENT,
    });
    expect(storeResult).toBeDefined();

    const recallResult = await callCoreRpc<unknown>('openhuman.memory_recall_memories', {
      namespace: TEST_NAMESPACE,
      limit: 10,
    });
    const recalled = JSON.stringify(recallResult ?? {});
    expect(recalled.includes(TEST_KEY) || recalled.includes(TEST_CONTENT)).toBe(true);
  });

  test('cross-chat retrieval path succeeds for a different namespace', async () => {
    const nsA = 'e2e-memory-chat-a-773';
    const nsB = 'e2e-memory-chat-b-773';
    const factKey = 'phoenix-landing-fact';
    const factContent = 'Phoenix migration landing confirmed for Friday evening. E2E canary #773';

    await callCoreRpc<unknown>('openhuman.memory_clear_namespace', { namespace: nsA });
    await callCoreRpc<unknown>('openhuman.memory_clear_namespace', { namespace: nsB });

    await callCoreRpc<unknown>('openhuman.memory_doc_put', {
      namespace: nsA,
      key: factKey,
      title: 'Phoenix landing fact',
      content: factContent,
    });

    const recallResult = await callCoreRpc<unknown>('openhuman.memory_recall_memories', {
      namespace: nsB,
      limit: 20,
    });
    expect(typeof recallResult).not.toBe('undefined');

    await callCoreRpc<unknown>('openhuman.memory_clear_namespace', { namespace: nsA });
    await callCoreRpc<unknown>('openhuman.memory_clear_namespace', { namespace: nsB });
  });

  test('clears a namespace and recall no longer returns the canary', async () => {
    await callCoreRpc<unknown>('openhuman.memory_doc_put', {
      namespace: TEST_NAMESPACE,
      key: TEST_KEY,
      title: TEST_TITLE,
      content: TEST_CONTENT,
    });

    await callCoreRpc<unknown>('openhuman.memory_clear_namespace', { namespace: TEST_NAMESPACE });

    const recallAfterForget = await callCoreRpc<unknown>('openhuman.memory_recall_memories', {
      namespace: TEST_NAMESPACE,
      limit: 10,
    });
    let recalled = JSON.stringify(recallAfterForget ?? {});
    if (recalled.includes(TEST_KEY) || recalled.includes(TEST_CONTENT)) {
      await new Promise(resolve => setTimeout(resolve, 3_000));
      const retry = await callCoreRpc<unknown>('openhuman.memory_recall_memories', {
        namespace: TEST_NAMESPACE,
        limit: 10,
      });
      recalled = JSON.stringify(retry ?? {});
    }
    expect(recalled.includes(TEST_KEY)).toBe(false);
    expect(recalled.includes(TEST_CONTENT)).toBe(false);
  });
});
