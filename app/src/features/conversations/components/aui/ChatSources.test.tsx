/**
 * Inline source list under a settled answer.
 *
 * Sources travel as assistant-ui `source` parts (`assistantParts`) and `Thread`
 * groups them into its `SourceGroup` slot, which `/chat` fills with
 * `ChatSources`.
 *
 * Five things are under test, and the second is the one that matters:
 *
 * 1. the list renders the turn's `http(s)` sources;
 * 2. it is actually **reached from the live `/chat` surface** — mounted through
 *    `AssistantUiChat`, with the trail arriving by the real route (the derived
 *    transcript RPC → `mapDisplayItems` → the adapter → message metadata), not
 *    by rendering `ChatSources` directly with a hand-made prop. A component that
 *    renders correctly in isolation while nothing mounts it is the defect shape
 *    this codebase keeps producing (the old voice-only transcript panel, the
 *    assistant-ui reasoning part, the suggestion chips), so proving the wiring is the point;
 * 3. a non-`http(s)` URL never becomes a link. Sources are derived from the
 *    `url` argument of a fetch tool call, which is raw model output, so a
 *    `javascript:` value must not reach an `<a href>`. `extractAgentSources`
 *    enforces that; this pins that the enforcement survives the trip through
 *    the inline surface;
 * 4. the turn is drawn once. A settled answer used to carry a second summary
 *    of its own reasoning and tools under it (a "N steps · M tools" footer and
 *    a sources list both read from a duplicate `processTrail`); only the inline
 *    parts remain;
 * 5. a memory citation on the message's `extraMetadata.citations`
 *    (`ChatDoneEvent.citations` / `ChatSegmentEvent.citations`) renders
 *    alongside the `url` sources as a `document` source badge, with no href.
 *
 * Every row renders through the vendored `sources.aui` element's primitives
 * directly. A short list (up to `MAX_INLINE_SOURCES`) is inline with no expand
 * step; a longer one is grouped behind a single "N sources" toggle — a
 * research turn's twenty-odd badges used to wrap across half the screen.
 *
 * Only the RPC is stubbed — the boundary a unit test should stub. Everything
 * between it and the DOM is production code.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../../../services/api/threadApi';
import chatRuntimeReducer from '../../../../store/chatRuntimeSlice';
import mascotReducer from '../../../../store/mascotSlice';
import runModeReducer from '../../../../store/runModeSlice';
import threadGoalReducer from '../../../../store/threadGoalSlice';
import threadReducer from '../../../../store/threadSlice';
import threadTodosReducer from '../../../../store/threadTodosSlice';
import type { DerivedDisplayItem } from '../../../../types/derivedTranscript';
import type { ThreadMessage } from '../../../../types/thread';
import { AssistantUiChat } from '../AssistantUiChat';
import { ChatSources, MAX_INLINE_SOURCES, sourceDomain } from './ChatSources';

const THREAD_ID = 't-sources';
const REQUEST_ID = 'req-sources';
const ANSWER = 'Here is what those pages say.';

function toolCall(callId: string, url: string): DerivedDisplayItem {
  return { kind: 'toolCall', callId, name: 'web_fetch', args: { url }, status: 'success' };
}

/**
 * A transcript page as the RPC returns one: **newest-first**.
 * `mapDisplayItems` reverses it, so the turn boundary has to sit last here to
 * end up first chronologically — otherwise the tool calls anchor to no turn.
 */
function page(...newestFirst: DerivedDisplayItem[]) {
  return {
    items: [...newestFirst, { kind: 'turnBoundary', requestId: REQUEST_ID } as DerivedDisplayItem],
    hasTranscript: true,
    hasMore: false,
  };
}

function agentMessage(citations?: unknown[]): ThreadMessage {
  return {
    id: 'm-1',
    content: ANSWER,
    type: 'text',
    extraMetadata: { requestId: REQUEST_ID, ...(citations ? { citations } : {}) },
    sender: 'agent',
    createdAt: '2026-01-01T00:00:00.000Z',
  };
}

function buildStore(message: ThreadMessage = agentMessage()) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      threadGoal: threadGoalReducer,
      threadTodos: threadTodosReducer,
      chatRuntime: chatRuntimeReducer,
      mascot: mascotReducer,
      runMode: runModeReducer,
    }),
    preloadedState: {
      thread: {
        threads: [
          {
            id: THREAD_ID,
            title: 'Sources thread',
            chatId: null,
            isActive: false,
            messageCount: 1,
            lastMessageAt: '2026-01-01T00:00:00.000Z',
            createdAt: '2026-01-01T00:00:00.000Z',
            labels: [],
          },
        ],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [THREAD_ID]: [message] },
        messages: [message],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

/** Mounted exactly as `/chat` mounts it — never `<ChatSources />` directly. */
function renderChat(message?: ThreadMessage) {
  return render(
    <Provider store={buildStore(message)}>
      <AssistantUiChat
        model={null}
        onModelChange={vi.fn()}
        inputValue=""
        onInputValueChange={vi.fn()}
        attachments={[]}
        onAttachFiles={vi.fn()}
        onRemoveAttachment={vi.fn()}
        maxAttachments={5}
        attachmentsEnabled={false}
        attachmentInteractionBlocked={false}
        onAttachmentOnlySend={vi.fn()}
      />
    </Provider>
  );
}

function sourceHrefs(): (string | null)[] {
  return Array.from(
    document.querySelectorAll<HTMLAnchorElement>('[data-testid="agent-source-row"]')
  ).map(anchor => anchor.getAttribute('href'));
}

beforeEach(() => {
  vi.restoreAllMocks();
});

describe('inline turn sources', () => {
  it('lists the turn sources on the live chat surface', async () => {
    vi.spyOn(threadApi, 'getDerivedTranscript').mockResolvedValue(
      page(toolCall('c2', 'https://docs.rs/b'), toolCall('c1', 'https://example.com/a')) as never
    );

    renderChat();

    // Reached through `AssistantUiChat` -> `Thread` -> the `SourceGroup` slot,
    // so this proves the wiring and not merely the component.
    await waitFor(() => expect(screen.getByTestId('turn-sources')).toBeTruthy());

    // No disclosure to open: every source badge is in the DOM already.
    expect(sourceHrefs()).toEqual(['https://example.com/a', 'https://docs.rs/b']);
  });

  it('renders a memory citation alongside url sources, with no href', async () => {
    vi.spyOn(threadApi, 'getDerivedTranscript').mockResolvedValue(
      page(toolCall('c1', 'https://example.com/a')) as never
    );

    renderChat(
      agentMessage([
        {
          id: 'cite-1',
          key: 'user_timezone',
          namespace: 'profile',
          timestamp: '2026-01-01T00:00:00.000Z',
          snippet: 'User is in UTC+2.',
        },
      ])
    );

    await waitFor(() => expect(screen.getByTestId('turn-sources')).toBeTruthy());
    expect(screen.getByTestId('agent-memory-source-row')).toBeTruthy();
    expect(screen.getByText('user_timezone')).toBeTruthy();
    expect(sourceHrefs()).toEqual(['https://example.com/a']);
  });

  it('draws the turn once, with no process footer under the answer', async () => {
    vi.spyOn(threadApi, 'getDerivedTranscript').mockResolvedValue(
      page(toolCall('c1', 'https://example.com/a'), {
        kind: 'reasoning',
        text: 'Looking it up.',
      } as DerivedDisplayItem) as never
    );

    renderChat();

    await waitFor(() => expect(screen.getByTestId('turn-sources')).toBeTruthy());
    // One activity disclosure (reasoning + tools, collapsed once settled) and
    // nothing summarising it a second time under the answer.
    expect(document.querySelectorAll('[data-slot="tool-group-root"]')).toHaveLength(1);
    expect(document.querySelector('[data-testid="turn-process-footer"]')).toBeNull();
    expect(screen.queryByText(/\d+ steps? ·/)).toBeNull();
  });

  it('renders nothing when the turn visited no sources', async () => {
    vi.spyOn(threadApi, 'getDerivedTranscript').mockResolvedValue(page() as never);

    renderChat();

    // Guarded so the absence cannot pass on a blank tree: the answer has to be
    // on screen for this assertion to mean anything.
    await waitFor(() => expect(screen.getByText(ANSWER)).toBeTruthy());
    expect(document.querySelector('[data-testid="turn-sources"]')).toBeNull();
  });

  it('never renders a non-http(s) url as a link', async () => {
    vi.spyOn(threadApi, 'getDerivedTranscript').mockResolvedValue(
      page(
        toolCall('c2', 'https://example.com/safe'),
        toolCall('c1', 'javascript:alert(1)')
      ) as never
    );

    renderChat();

    await waitFor(() => expect(screen.getByTestId('turn-sources')).toBeTruthy());

    // One row, not two: the `javascript:` entry is dropped by
    // `extractAgentSources`, so it is never counted and never linked.
    expect(sourceHrefs()).toEqual(['https://example.com/safe']);
    // The fetch card may show the raw argument as text; it must never be a link.
    const hrefs = Array.from(document.querySelectorAll('a[href]')).map(a => a.getAttribute('href'));
    expect(hrefs.some(href => href?.startsWith('javascript:'))).toBe(false);
  });

  it('groups a long source list behind one summary toggle', async () => {
    const urls = Array.from(
      { length: MAX_INLINE_SOURCES + 3 },
      (_, i) => `https://site${i}.example.com/page`
    );
    vi.spyOn(threadApi, 'getDerivedTranscript').mockResolvedValue(
      page(...urls.map((url, i) => toolCall(`c${i}`, url)).reverse()) as never
    );

    renderChat();

    const toggle = await screen.findByTestId('turn-sources-toggle');
    // Collapsed: one summary chip, not a wall of badges.
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(toggle.textContent).toContain(`${urls.length} sources`);
    expect(sourceHrefs()).toEqual([]);

    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(sourceHrefs()).toEqual(urls);

    fireEvent.click(toggle);
    expect(sourceHrefs()).toEqual([]);
  });

  it('keeps a short source list inline with no toggle', async () => {
    vi.spyOn(threadApi, 'getDerivedTranscript').mockResolvedValue(
      page(toolCall('c1', 'https://example.com/a')) as never
    );

    renderChat();

    await waitFor(() => expect(screen.getByTestId('turn-sources')).toBeTruthy());
    expect(screen.queryByTestId('turn-sources-toggle')).toBeNull();
    expect(sourceHrefs()).toEqual(['https://example.com/a']);
  });
});

describe('source favicons', () => {
  const REDIRECT = 'https://vertexaisearch.cloud.google.com/grounding-api-redirect/AbC123';

  it('keys the favicon on a grounding redirect target domain, not the redirect host', () => {
    render(
      <ChatSources
        sources={[
          { id: 's1', sourceType: 'url', url: REDIRECT, title: 'reddit.com' },
          { id: 's2', sourceType: 'url', url: 'https://www.docs.rs/x', title: 'A page title' },
        ]}
      />
    );

    const icons = Array.from(
      document.querySelectorAll<HTMLImageElement>('[data-slot="source-icon"]')
    ).map(img => img.getAttribute('src'));
    expect(icons).toEqual([
      'https://icons.duckduckgo.com/ip3/reddit.com.ico',
      'https://icons.duckduckgo.com/ip3/docs.rs.ico',
    ]);
    // The link itself still goes where the source said.
    expect(sourceHrefs()).toEqual([REDIRECT, 'https://www.docs.rs/x']);
  });

  it('falls back to the url host when the title is not a hostname', () => {
    expect(
      sourceDomain({ id: 'a', sourceType: 'url', url: 'https://www.x.org/p', title: 'X' })
    ).toBe('x.org');
    expect(
      sourceDomain({ id: 'b', sourceType: 'url', url: REDIRECT, title: 'WWW.Reddit.com' })
    ).toBe('reddit.com');
  });
});
