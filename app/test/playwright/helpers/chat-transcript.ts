import { expect, type Page } from '@playwright/test';

import { waitForSelectedThreadId } from './chat-drive';
import { callCoreRpc } from './core-rpc';

export interface PersistedThreadMessage {
  content: string;
  sender: string;
  extraMetadata: Record<string, unknown>;
}

export async function persistedThreadMessages(threadId: string): Promise<PersistedThreadMessage[]> {
  let response = await callCoreRpc<unknown>('openhuman.threads_messages_list', {
    thread_id: threadId,
  });
  for (let depth = 0; depth < 3; depth += 1) {
    if (!response || typeof response !== 'object' || !('result' in response)) break;
    response = (response as { result: unknown }).result;
  }
  expect(response && typeof response === 'object' && 'data' in response).toBe(true);
  const data = (response as { data: unknown }).data;
  expect(data && typeof data === 'object' && 'messages' in data).toBe(true);
  const messages = (data as { messages: unknown }).messages;
  expect(Array.isArray(messages)).toBe(true);
  return messages as PersistedThreadMessage[];
}

export async function selectedThreadTranscript(
  page: Page
): Promise<{ threadId: string; content: string; total: number }> {
  const threadId = await waitForSelectedThreadId(page);
  const response = await callCoreRpc<unknown>('openhuman.threads_transcript_get', {
    thread_id: threadId,
    limit: 500,
  });
  let envelope = response;
  for (let depth = 0; depth < 3; depth += 1) {
    if (!envelope || typeof envelope !== 'object' || !('result' in envelope)) break;
    envelope = (envelope as { result: unknown }).result;
  }
  expect(envelope && typeof envelope === 'object' && 'data' in envelope).toBe(true);
  const data = (envelope as { data: unknown }).data;
  expect(data && typeof data === 'object' && 'hasTranscript' in data).toBe(true);
  expect((data as { hasTranscript: unknown }).hasTranscript).toBe(true);
  expect((data as { threadId: unknown }).threadId).toBe(threadId);
  expect((data as { hasMore: unknown }).hasMore).toBe(false);
  expect(Array.isArray((data as { items: unknown }).items)).toBe(true);
  expect((data as { total: number }).total).toBeGreaterThan(0);
  return { threadId, content: JSON.stringify(data), total: (data as { total: number }).total };
}

export async function inFlightThreadKeys(): Promise<string[]> {
  const response = await callCoreRpc<unknown>('openhuman.test_support_in_flight_chats', {});
  let status = response;
  for (let depth = 0; depth < 3; depth += 1) {
    if (!status || typeof status !== 'object' || !('result' in status)) break;
    status = (status as { result: unknown }).result;
  }
  expect(status && typeof status === 'object' && 'entries' in status).toBe(true);
  const entries = (status as { entries: unknown }).entries;
  expect(Array.isArray(entries)).toBe(true);
  return (entries as Array<{ key: string }>).map(entry => entry.key);
}

export async function inFlightThreadKey(threadId: string): Promise<string | undefined> {
  expect(threadId.length).toBeGreaterThan(0);
  const keys = await inFlightThreadKeys();
  return keys.find(key => key === threadId || key.endsWith(threadId));
}
