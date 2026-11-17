/**
 * Drive a chat turn the way the product does, without touching the composer.
 *
 * Every turn in these specs goes out over `openhuman.channel_web_chat` — the
 * same RPC `chatService.sendChatMessage` calls (`src/services/chatService.ts:1631`).
 * Two reasons, and only the second is about a bug:
 *
 *  1. It separates what is under test from how the text got there. A spec about
 *     stream routing or a parked gate should not fail because the composer
 *     changed, and a composer regression should not be reported as a routing
 *     one.
 *  2. `ComposerTextBridge` used to chain a synchronous `setState` per keystroke
 *     past React's nested-update limit, so an automated driver typing at speed
 *     took the chat surface to its error boundary
 *     (`src/features/conversations/components/AssistantUiChat.composerSync.test.tsx`).
 *     That is fixed and pinned by a unit test, but nothing here needs to depend
 *     on it staying fixed.
 *
 * The one thing the RPC cannot invent is `client_id`: the core routes the
 * stream to the socket that id belongs to, so a made-up one runs the turn and
 * delivers it nowhere this page can see. We read the live socket id out of the
 * store (`src/store/socketSlice.ts`, exposed as `window.__OPENHUMAN_STORE__`),
 * and send from inside the page so the request carries the renderer's own
 * credentials.
 */
import { expect, type Page } from '@playwright/test';

const MOCK_ADMIN_BASE = `http://127.0.0.1:${process.env.E2E_MOCK_PORT || '18473'}`;

interface MockRequestEntry {
  method: string;
  url: string;
  body: string;
  timestamp: number;
}

/** Reset the mock backend's behaviours, state and request log. */
export async function resetMock(): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/reset`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({}),
  });
}

/**
 * Set one behaviour key. The `{ key, value }` form is a single-key set on the
 * admin route (`scripts/mock-api/admin.mjs:99`) and is what every other spec in
 * this suite uses; the `{ behavior: {...} }` form merges a whole object.
 */
export async function setMockBehavior(key: string, value: string): Promise<void> {
  await fetch(`${MOCK_ADMIN_BASE}/__admin/behavior`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key, value }),
  });
}

/**
 * Script the mock LLM by keyword (`scripts/mock-api/routes/llm.mjs:505-560`).
 * A rule may carry `toolCalls`, in which case the mock answers with
 * `finish_reason: "tool_calls"` — which is how a spec makes the agent call a
 * specific tool without a real model.
 */
export async function setKeywordRules(rules: unknown[]): Promise<void> {
  await setMockBehavior('llmKeywordRules', JSON.stringify(rules));
}

/** Every request the mock has served since the last reset, newest last. */
export async function mockRequests(): Promise<MockRequestEntry[]> {
  const res = await fetch(`${MOCK_ADMIN_BASE}/__admin/requests`);
  const payload = (await res.json()) as { data?: MockRequestEntry[] };
  return payload.data ?? [];
}

/** Upstream request bodies the mock has seen, as raw strings. */
export async function upstreamBodies(): Promise<string[]> {
  return (await mockRequests()).map(entry => entry.body).filter(body => Boolean(body));
}

/**
 * The socket id the core must route this page's streams to.
 *
 * `socket.byUser` is keyed by user id and can hold a `__pending__` entry before
 * the user resolves, so we take the first entry that is actually connected
 * rather than assuming a key.
 */
export async function connectedSocketId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState?: () => {
            socket?: { byUser?: Record<string, { status?: string; socketId?: string | null }> };
          };
        };
      }
    ).__OPENHUMAN_STORE__;
    const byUser = store?.getState?.().socket?.byUser ?? {};
    for (const entry of Object.values(byUser)) {
      if (entry?.status === 'connected' && entry.socketId) return entry.socketId;
    }
    return null;
  });
}

export async function waitForConnectedSocketId(page: Page, timeout = 30_000): Promise<string> {
  await expect
    .poll(async () => connectedSocketId(page), {
      timeout,
      message: 'the renderer never reported a connected socket, so no turn could be routed to it',
    })
    .not.toBeNull();
  return (await connectedSocketId(page)) as string;
}

/** The thread the UI currently has selected. */
export async function selectedThreadId(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          getState?: () => { thread?: { selectedThreadId?: string | null } };
        };
      }
    ).__OPENHUMAN_STORE__;
    return store?.getState?.().thread?.selectedThreadId ?? null;
  });
}

export async function waitForSelectedThreadId(page: Page, timeout = 20_000): Promise<string> {
  await expect.poll(async () => selectedThreadId(page), { timeout }).not.toBeNull();
  return (await selectedThreadId(page)) as string;
}

/**
 * Call any core RPC from inside the page, with the renderer's own URL and
 * bearer token (seeded into localStorage by `seedBrowserCoreMode`).
 *
 * Deliberately not the Node-side `callCoreRpc` from `core-rpc.ts`: a turn sent
 * from Node would be a different client, and the stream would never reach this
 * page.
 */
export async function callRpcFromPage<T = unknown>(
  page: Page,
  method: string,
  params: Record<string, unknown>
): Promise<T> {
  return page.evaluate(
    async ({ method, params }) => {
      const url = window.localStorage.getItem('openhuman_core_rpc_url');
      const token = window.localStorage.getItem('openhuman_core_rpc_token');
      if (!url || !token) throw new Error('page has no core RPC url/token in localStorage');
      const response = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
        body: JSON.stringify({ jsonrpc: '2.0', id: Date.now(), method, params }),
      });
      const payload = (await response.json()) as { result?: unknown; error?: { message?: string } };
      if (payload.error) throw new Error(`RPC ${method} failed: ${payload.error.message}`);
      return payload.result;
    },
    { method, params }
  ) as Promise<T>;
}

export interface SendOptions {
  /** 'interrupt' (default), 'steer', 'followup', 'collect', or 'parallel'. */
  queueMode?: string;
  runMode?: 'plan' | 'build';
}

/**
 * Arm the shared turn-lifecycle entry for `threadId`, the way a real send does.
 *
 * Without this, every surface gated on `s.thread.isRunning` is invisible to a
 * driven turn, and the turn itself looks fine — tokens stream, the answer
 * renders, only the "something is running" chrome never appears.
 *
 * The chain: `useOpenHumanExternalStore.ts:404` derives `isRunning` from
 * `chatRuntime.inferenceTurnLifecycleByThread`, which is written in exactly two
 * places. `beginInferenceTurn` CREATES the entry and is dispatched client-side
 * on send (`Conversations.tsx:1161`) — not by anything the core emits.
 * `markInferenceTurnStreaming`, which the socket's `inference_start` drives
 * (`ChatRuntimeProvider.tsx:750`), only UPDATES an entry that already exists
 * (`chatRuntimeSlice.ts:2400` guards on it). So an RPC-driven turn creates no
 * entry, the socket event is a no-op against it, and `isRunning` stays `false`
 * for the whole turn.
 *
 * `useWorkflowBuilderChat.ts:427-439` hit this first and fixed it the same way,
 * and its comment is the clearest statement of the mechanism in the codebase.
 *
 * Dispatched as a plain action object because the slice's action creators are
 * not on `window`; the type string is `<slice name>/<reducer>` and both halves
 * are pinned by `chat-drive.test.ts`, so a rename cannot silently turn this
 * into a no-op dispatch that Redux ignores.
 */
async function armTurnLifecycle(page: Page, threadId: string): Promise<void> {
  const armed = await page.evaluate(threadId => {
    const store = (
      window as unknown as {
        __OPENHUMAN_STORE__?: {
          dispatch?: (action: unknown) => void;
          getState?: () => {
            chatRuntime?: { inferenceTurnLifecycleByThread?: Record<string, string> };
          };
        };
      }
    ).__OPENHUMAN_STORE__;
    if (!store?.dispatch || !store.getState) return false;
    store.dispatch({ type: 'chatRuntime/beginInferenceTurn', payload: { threadId } });
    const lifecycles = store.getState().chatRuntime?.inferenceTurnLifecycleByThread ?? {};
    return lifecycles[threadId] === 'started';
  }, threadId);

  // A dispatch Redux did not recognise is silently ignored, which would put
  // this helper right back where it started while looking like it worked.
  // Read the state back instead of trusting the dispatch.
  expect(
    armed,
    'beginInferenceTurn did not reach chatRuntime.inferenceTurnLifecycleByThread; ' +
      'the action type or slice name has changed'
  ).toBe(true);
}

/**
 * Send `message` on `threadId` as this page's user, and return once the core
 * has accepted it. The turn then streams to this page over the socket exactly
 * as a typed message would.
 */
export async function sendTurn(
  page: Page,
  threadId: string,
  message: string,
  options: SendOptions = {}
): Promise<void> {
  const clientId = await waitForConnectedSocketId(page);
  // Before the RPC, mirroring the order a real send uses
  // (`Conversations.tsx:1161` dispatches, then calls the service): the socket
  // can answer faster than the next `page.evaluate` round trip, and
  // `markInferenceTurnStreaming` is a no-op against a thread with no entry.
  await armTurnLifecycle(page, threadId);
  await callRpcFromPage(page, 'openhuman.channel_web_chat', {
    client_id: clientId,
    thread_id: threadId,
    message,
    source: 'type',
    ...(options.queueMode ? { queue_mode: options.queueMode } : {}),
    ...(options.runMode ? { run_mode: options.runMode } : {}),
  });
}

/** Create a thread from the sidebar and return its id. */
export async function startNewThread(page: Page): Promise<string> {
  const previous = await selectedThreadId(page);
  const sidebar = page.getByTestId('new-thread-sidebar-button');
  if (await sidebar.isVisible().catch(() => false)) {
    // The caller is exercising thread state, not overlay actionability. A
    // transient walkthrough/modal can cover this stable sidebar control.
    await sidebar.click({ force: true });
  } else {
    await page.getByTestId('new-thread-button').click({ force: true });
  }
  await expect.poll(async () => selectedThreadId(page), { timeout: 20_000 }).not.toBe(previous);
  return waitForSelectedThreadId(page);
}
