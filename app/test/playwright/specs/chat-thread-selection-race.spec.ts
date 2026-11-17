import { expect, test } from '@playwright/test';

import { selectedThreadId, startNewThread } from '../helpers/chat-drive';
import {
  bootAuthenticatedPage,
  dismissWalkthroughIfPresent,
  waitForAppReady,
} from '../helpers/core-rpc';

test('a stale thread-list response preserves a thread selected while it was in flight', async ({
  page,
}) => {
  await bootAuthenticatedPage(page, 'pw-thread-selection-race', '/chat');
  await waitForAppReady(page);
  await dismissWalkthroughIfPresent(page);

  const selectedDuringLoad = await startNewThread(page);
  const remainingThread = await startNewThread(page);
  const omittedThread = await startNewThread(page);
  expect(new Set([selectedDuringLoad, remainingThread, omittedThread]).size).toBe(3);

  let captureRequest: (() => void) | undefined;
  const requestCaptured = new Promise<void>(resolve => {
    captureRequest = resolve;
  });
  let releaseResponse: (() => void) | undefined;
  const responseReleased = new Promise<void>(resolve => {
    releaseResponse = resolve;
  });
  let releaseConcurrentThreadLists: (() => void) | undefined;
  const concurrentThreadListsReleased = new Promise<void>(resolve => {
    releaseConcurrentThreadLists = resolve;
  });
  let holdNextThreadList = false;
  let deferConcurrentThreadLists = false;
  const responseMarker = '__stale_thread_list_applied__';

  await page.route('**/rpc', async (route, request) => {
    const body = JSON.parse(request.postData() || '{}') as { id: number; method: string };
    if (body.method !== 'openhuman.threads_list') {
      await route.continue();
      return;
    }

    if (!holdNextThreadList) {
      if (deferConcurrentThreadLists) await concurrentThreadListsReleased;
      await route.continue();
      return;
    }

    const holdResponse = holdNextThreadList;
    holdNextThreadList = false;
    deferConcurrentThreadLists = true;
    const response = await route.fetch();
    const payload = (await response.json()) as {
      result: { data: { count: number; threads: Array<{ id: string; title?: string }> } };
    };
    if (!payload.result?.data?.threads) {
      throw new Error(`unexpected threads_list response: ${JSON.stringify(payload)}`);
    }
    const threadList = payload.result.data;
    payload.result.data = {
      ...threadList,
      threads: threadList.threads
        .filter(thread => thread.id !== selectedDuringLoad && thread.id !== omittedThread)
        .map(thread =>
          thread.id === remainingThread ? { ...thread, title: responseMarker } : thread
        ),
    };
    payload.result.data.count = payload.result.data.threads.length;
    if (holdResponse) {
      captureRequest?.();
      await responseReleased;
    }
    await route.fulfill({ response, body: JSON.stringify(payload) });
  });

  await page.goto('/#/settings/account');
  await waitForAppReady(page);
  holdNextThreadList = true;
  await page.goto('/#/chat');
  await expect(page.getByTestId('chat-message-input')).toBeVisible();
  await requestCaptured;

  await page.getByTestId(`thread-row-${selectedDuringLoad}`).click({ force: true });
  await expect.poll(() => selectedThreadId(page)).toBe(selectedDuringLoad);

  releaseResponse?.();
  await expect
    .poll(() =>
      page.evaluate(
        ({
          selectedThreadId,
          omittedThreadId,
          remainingThreadId,
        }: {
          selectedThreadId: string;
          omittedThreadId: string;
          remainingThreadId: string;
        }) => {
          const store = (
            window as typeof window & {
              __OPENHUMAN_STORE__?: {
                getState?: () => { thread?: { threads?: Array<{ id: string; title?: string }> } };
              };
            }
          ).__OPENHUMAN_STORE__;
          const threads = store?.getState?.().thread?.threads ?? [];
          return {
            selectedThreadPreserved: threads.some(thread => thread.id === selectedThreadId),
            omittedThreadRemoved: !threads.some(thread => thread.id === omittedThreadId),
            filteredResponseApplied: threads.some(
              thread =>
                thread.id === remainingThreadId && thread.title === '__stale_thread_list_applied__'
            ),
          };
        },
        {
          selectedThreadId: selectedDuringLoad,
          omittedThreadId: omittedThread,
          remainingThreadId: remainingThread,
        }
      )
    )
    .toEqual({
      selectedThreadPreserved: true,
      omittedThreadRemoved: true,
      filteredResponseApplied: true,
    });
  releaseConcurrentThreadLists?.();
  expect(await selectedThreadId(page)).toBe(selectedDuringLoad);
});
