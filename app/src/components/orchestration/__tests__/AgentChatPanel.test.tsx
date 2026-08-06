import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../../lib/orchestration/orchestrationClient';
import AgentChatPanel from '../AgentChatPanel';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

// The welcome hero pulls Redux/user/usage state; stub it so the panel renders
// without the app providers. We only assert it mounts on an empty conscious thread.
vi.mock('../../chat/ChatNewWindowHero', () => ({
  default: () => <div data-testid="chat-new-window-hero" />,
}));

const selectChat = vi.hoisted(() => vi.fn());
const sendMessage = vi.hoisted(() => vi.fn().mockResolvedValue(true));
const chatsApi = vi.hoisted(() => ({
  current: {
    sessionsState: { status: 'ok' as const },
    messagesState: { status: 'ok' as const },
    chats: [
      { id: 'master', title: 'Master', subtitle: 'you', unread: 0, messages: [] as unknown[] },
      {
        id: 'subconscious',
        title: 'Subconscious',
        subtitle: 'loop',
        unread: 0,
        messages: [] as unknown[],
      },
    ],
    selectedId: 'master',
    selected: { id: 'master', title: 'Master', messages: [] as unknown[] },
    status: null as unknown,
    masterError: null as string | null,
    selectChat,
    refresh: vi.fn(),
    sendMessage,
  },
}));
vi.mock('../../../lib/orchestration/useOrchestrationChats', () => ({
  MASTER_CHAT_KEY: 'master',
  SUBCONSCIOUS_CHAT_KEY: 'subconscious',
  useOrchestrationChats: () => chatsApi.current,
}));

const subconsciousTrigger = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
vi.mock('../../../utils/tauriCommands/subconscious', () => ({ subconsciousTrigger }));

const sendMasterMessage = vi.hoisted(() => vi.fn().mockResolvedValue({ ok: true, messageId: 'm' }));
vi.mock('../../../lib/orchestration/orchestrationClient', async orig => ({
  ...(await orig<typeof import('../../../lib/orchestration/orchestrationClient')>()),
  orchestrationClient: { sendMasterMessage },
}));

const contactSessions = vi.hoisted(() => ({ current: [] as SessionSummary[] }));
const transcript = vi.hoisted(() => ({
  current: { state: { status: 'ok' as const }, messages: [] as unknown[], refresh: vi.fn() },
}));
vi.mock('../../../lib/orchestration/useOrchestrationSessions', () => ({
  useContactSessions: () => ({
    state: { status: 'ok' },
    sessions: contactSessions.current,
    byContact: new Map(),
    refresh: vi.fn(),
  }),
  useSessionTranscript: () => transcript.current,
}));

const pinged: SessionSummary = {
  sessionId: 's-auth',
  agentId: '@peer',
  source: 'claude',
  status: 'waiting-approval',
  chatKind: 'session',
  lastMessageAt: '2026-07-08T00:00:00Z',
  unread: 0,
  active: true,
  pinned: false,
  label: 'auth-fix',
  messageCount: 3,
};

describe('AgentChatPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    contactSessions.current = [];
    transcript.current = { state: { status: 'ok' }, messages: [], refresh: vi.fn() };
    chatsApi.current = {
      ...chatsApi.current,
      selectedId: 'master',
      selected: { id: 'master', title: 'Master', messages: [] },
      masterError: null,
    };
  });

  it('renders the conscious/subconscious toggle and switches conversation', () => {
    render(<AgentChatPanel />);
    const conscious = screen.getByTestId('orch-agent-tab-master');
    expect(conscious).toHaveAttribute('role', 'radio');
    expect(conscious).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(screen.getByTestId('orch-agent-tab-subconscious'));
    expect(selectChat).toHaveBeenCalledWith('subconscious');
  });

  it('shows the welcome hero on an empty conscious thread', () => {
    render(<AgentChatPanel />);
    expect(screen.getByTestId('chat-new-window-hero')).toBeInTheDocument();
  });

  it('sends a master message from the composer', async () => {
    render(<AgentChatPanel />);
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'go' } });
    fireEvent.click(screen.getByTestId('send-message-button'));
    await waitFor(() =>
      expect(sendMessage).toHaveBeenCalledWith(expect.objectContaining({ id: 'master' }), 'go')
    );
  });

  it('shows the steering header + runs a review on the subconscious thread', () => {
    chatsApi.current = {
      ...chatsApi.current,
      selectedId: 'subconscious',
      selected: { id: 'subconscious', title: 'Subconscious', messages: [] },
    };
    render(<AgentChatPanel />);
    expect(screen.getByTestId('orch-agent-steering')).toBeInTheDocument();
    fireEvent.click(screen.getByText('tinyplaceOrchestration.steeringHeader.runReview'));
    expect(subconsciousTrigger).toHaveBeenCalledWith('all');
  });

  it('opens a session subpage from a View-session card and replies', async () => {
    contactSessions.current = [pinged];
    render(<AgentChatPanel />);
    expect(screen.queryByTestId('orch-session-header')).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId('orch-agent-view-session-s-auth'));
    expect(screen.getByTestId('orch-session-header')).toBeInTheDocument();

    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'hi' } });
    fireEvent.click(screen.getByTestId('send-message-button'));
    await waitFor(() =>
      expect(sendMasterMessage).toHaveBeenCalledWith({
        body: 'hi',
        recipient: '@peer',
        sessionId: 's-auth',
      })
    );
  });

  it('routes a runtime tool-approval decision back as an allow reply', async () => {
    contactSessions.current = [pinged];
    transcript.current = {
      state: { status: 'ok' },
      messages: [
        {
          id: 'ap',
          from: 'agent',
          body: 'gh pr status',
          timestamp: '2026-07-08T00:00:00Z',
          encrypted: false,
          eventKind: 'approval_request',
          toolName: 'shell',
        },
      ],
      refresh: vi.fn(),
    };
    render(<AgentChatPanel />);
    fireEvent.click(screen.getByTestId('orch-agent-view-session-s-auth'));
    fireEvent.click(screen.getByText('chat.approval.approve'));
    await waitFor(() =>
      expect(sendMasterMessage).toHaveBeenCalledWith({
        body: 'allow',
        recipient: '@peer',
        sessionId: 's-auth',
      })
    );
  });

  it('surfaces a session reply failure', async () => {
    contactSessions.current = [pinged];
    sendMasterMessage.mockRejectedValueOnce(new Error('boom'));
    render(<AgentChatPanel />);
    fireEvent.click(screen.getByTestId('orch-agent-view-session-s-auth'));
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'hi' } });
    fireEvent.click(screen.getByTestId('send-message-button'));
    expect(await screen.findByTestId('orch-session-reply-error')).toHaveTextContent('boom');
  });

  it('shows an error state when the transcript fails to load', () => {
    chatsApi.current = {
      ...chatsApi.current,
      messagesState: { status: 'error', message: 'load failed' } as never,
    };
    render(<AgentChatPanel />);
    expect(screen.getByText(/load failed/)).toBeInTheDocument();
  });

  // ── Autoscroll (regression: new master message snapped to the TOP) ─────────
  const msg = (id: string) => ({
    id,
    from: 'you',
    body: id,
    timestamp: '2026-07-08T00:00:00Z',
    encrypted: false,
  });

  // jsdom does no layout, so `scrollTop`/`scrollHeight`/`clientHeight` are inert.
  // Back them with a stored value so the stick-to-bottom snap is observable.
  const fakeMetrics = (el: HTMLElement, scrollHeight: number, clientHeight: number) => {
    let top = 0;
    Object.defineProperty(el, 'scrollHeight', { value: scrollHeight, configurable: true });
    Object.defineProperty(el, 'clientHeight', { value: clientHeight, configurable: true });
    Object.defineProperty(el, 'scrollTop', {
      get: () => top,
      set: v => {
        top = v;
      },
      configurable: true,
    });
  };

  it('pins the newest master message to the bottom on a new message (not the top)', () => {
    chatsApi.current = {
      ...chatsApi.current,
      selectedId: 'master',
      selected: { id: 'master', title: 'Master', messages: [msg('m1')] },
    };
    const { rerender } = render(<AgentChatPanel />);
    const scroll = screen.getByTestId('orch-chat-scroll') as HTMLDivElement;
    fakeMetrics(scroll, 1000, 400);
    scroll.scrollTop = 0; // as if reset to the top by the loading-spinner swap

    chatsApi.current = {
      ...chatsApi.current,
      selected: { id: 'master', title: 'Master', messages: [msg('m1'), msg('m2')] },
    };
    rerender(<AgentChatPanel />);

    expect(scroll.scrollTop).toBe(1000); // snapped to the bottom, not left at 0
  });

  it('does not yank the master chat down when the user has scrolled up', () => {
    chatsApi.current = {
      ...chatsApi.current,
      selectedId: 'master',
      selected: { id: 'master', title: 'Master', messages: [msg('m1')] },
    };
    const { rerender } = render(<AgentChatPanel />);
    const scroll = screen.getByTestId('orch-chat-scroll') as HTMLDivElement;
    fakeMetrics(scroll, 1000, 400);
    scroll.scrollTop = 0; // 600px from the bottom, past the 80px threshold
    fireEvent.scroll(scroll); // disengages stickiness

    chatsApi.current = {
      ...chatsApi.current,
      selected: { id: 'master', title: 'Master', messages: [msg('m1'), msg('m2')] },
    };
    rerender(<AgentChatPanel />);

    expect(scroll.scrollTop).toBe(0); // left where the user parked it
  });

  it('renders with no selected chat (covers the empty-messages fallback)', () => {
    chatsApi.current = { ...chatsApi.current, selected: undefined as never };
    render(<AgentChatPanel />);
    // Exercises `selected?.messages ?? EMPTY_MESSAGES`; the panel still renders.
    expect(screen.getByTestId('orch-agent-tab-master')).toBeInTheDocument();
  });

  // ── Composer-footer measurement (regression: #5162 / TAURI-REACT-2G) ───────
  //
  // `ChatPageScaffold` measures its floating footer with a ResizeObserver and
  // feeds the height into the scroll region's bottom padding. The effect used to
  // depend on the `footer` *node*, which is inline JSX at every call site and so
  // takes a fresh identity on every render — the observer was torn down and
  // rebuilt each pass, and because `observe()` delivers an immediate
  // observation, every render scheduled another height update. Any measurement
  // that disagreed with the committed value re-rendered and re-ran the effect,
  // cascading until React aborted with "Maximum update depth exceeded". Typing
  // was the easiest trigger: the composer's auto-growing textarea lives inside
  // this footer.
  describe('composer footer measurement', () => {
    /** Model of the real ResizeObserver: `observe()` reports an initial size. */
    class MockResizeObserver {
      static instances: MockResizeObserver[] = [];
      observed: Element[] = [];
      disconnected = false;
      constructor(private readonly cb: () => void) {
        MockResizeObserver.instances.push(this);
      }
      observe(el: Element) {
        this.observed.push(el);
        this.cb();
      }
      unobserve() {}
      disconnect() {
        this.disconnected = true;
      }
      /** Drive a size change the way a real layout change would. */
      fire() {
        this.cb();
      }
    }

    /**
     * Observers watching the composer footer. `useStickToBottom` also builds a
     * ResizeObserver (on the scroll container), so the global mock catches both —
     * select ours by the element it observes.
     */
    const footerObservers = () =>
      MockResizeObserver.instances.filter(o =>
        o.observed.some(el => (el as HTMLElement).dataset?.testid === 'orch-chat-footer')
      );

    const footerHeight = { current: 96 };
    let originalResizeObserver: typeof globalThis.ResizeObserver | undefined;
    let originalOffsetHeight: PropertyDescriptor | undefined;

    beforeEach(() => {
      MockResizeObserver.instances = [];
      footerHeight.current = 96;
      originalResizeObserver = globalThis.ResizeObserver;
      globalThis.ResizeObserver = MockResizeObserver as unknown as typeof globalThis.ResizeObserver;
      // jsdom does no layout, so every `offsetHeight` is 0. Report a real height
      // for the footer wrapper only, so the measurement path is observable.
      originalOffsetHeight = Object.getOwnPropertyDescriptor(
        HTMLElement.prototype,
        'offsetHeight'
      ) as PropertyDescriptor | undefined;
      Object.defineProperty(HTMLElement.prototype, 'offsetHeight', {
        configurable: true,
        get(this: HTMLElement) {
          return this.dataset.testid === 'orch-chat-footer' ? footerHeight.current : 0;
        },
      });
    });

    afterEach(() => {
      if (originalResizeObserver) globalThis.ResizeObserver = originalResizeObserver;
      else delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
      if (originalOffsetHeight) {
        Object.defineProperty(HTMLElement.prototype, 'offsetHeight', originalOffsetHeight);
      } else {
        delete (HTMLElement.prototype as unknown as Record<string, unknown>).offsetHeight;
      }
    });

    it('does not rebuild the footer ResizeObserver on every render while typing', () => {
      render(<AgentChatPanel />);
      expect(footerObservers()).toHaveLength(1);

      // Each keystroke re-renders the panel, which produces a fresh `footer`
      // element. That must NOT re-subscribe the observer.
      const textbox = screen.getByRole('textbox');
      fireEvent.change(textbox, { target: { value: 'h' } });
      fireEvent.change(textbox, { target: { value: 'he' } });
      fireEvent.change(textbox, { target: { value: 'hey' } });

      expect(footerObservers()).toHaveLength(1);
      expect(footerObservers()[0].disconnected).toBe(false);
    });

    it('still reserves the measured footer height on the scroll region', () => {
      render(<AgentChatPanel />);
      const scroll = screen.getByTestId('orch-chat-scroll');
      expect(scroll.style.paddingBottom).toBe('96px');

      // A real layout change (the composer growing a line) must still be picked
      // up now that the effect subscribes on footer presence rather than identity.
      footerHeight.current = 132;
      act(() => footerObservers()[0].fire());

      expect(scroll.style.paddingBottom).toBe('132px');
      expect(footerObservers()).toHaveLength(1);
    });
  });
});
