/**
 * AgentChatPanel — chat with the main agent, rendered exactly like the app's
 * normal chat page.
 *
 * Full-bleed page-variant layout (dark background, centered width-capped message
 * column, a floating composer over a bottom fade, one vertical scroll) shared by
 * three views via {@link ChatPageScaffold}:
 *
 *   - the **conscious** master session (composer + welcome hero when empty),
 *   - the **subconscious** steering loop (steering header, no composer),
 *   - a **peer session subpage** — opened from the sidebar's active sub-agents
 *     list or an inline "needs you" card, it takes over the whole pane (not a
 *     side drawer) so the session's chat renders full-size, with a back link.
 *
 * Conscious/Subconscious is a bottom toggle in the composer footer (where the
 * generic chat's super-context / quick-reasoning controls sit).
 */
import debugFactory from 'debug';
import {
  type KeyboardEvent,
  type ReactNode,
  type Ref,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react';

import { useStickToBottom } from '../../hooks/useStickToBottom';
import { useT } from '../../lib/i18n/I18nContext';
import {
  orchestrationClient,
  type SessionSummary,
} from '../../lib/orchestration/orchestrationClient';
import {
  MASTER_CHAT_KEY,
  SUBCONSCIOUS_CHAT_KEY,
  useOrchestrationChats,
} from '../../lib/orchestration/useOrchestrationChats';
import {
  useContactSessions,
  useSessionTranscript,
} from '../../lib/orchestration/useOrchestrationSessions';
import { subconsciousTrigger } from '../../utils/tauriCommands/subconscious';
import ChatComposer from '../chat/ChatComposer';
import ChatNewWindowHero from '../chat/ChatNewWindowHero';
import Button from '../ui/Button';
import SessionTranscript from './SessionTranscript';

const debug = debugFactory('orchestration:agent-chat');

// Stable identity for an empty transcript so `useStickToBottom`'s layout effect
// doesn't re-run every render when the selected chat has no messages yet.
const EMPTY_MESSAGES: readonly unknown[] = [];

function sessionLabel(session: SessionSummary): string {
  return session.label?.trim() || session.sessionId;
}

/**
 * Page-variant chat scaffold: the normal chat window's dark surface, a
 * full-width scroll region with a centered width-capped body, a bottom fade, and
 * a floating composer footer. The footer's measured height reserves bottom
 * padding on the scroll region so the last message clears it.
 */
function ChatPageScaffold({
  header,
  footer,
  scrollRef,
  children,
}: {
  header?: ReactNode;
  footer?: ReactNode;
  scrollRef?: Ref<HTMLDivElement>;
  children: ReactNode;
}) {
  const footerRef = useRef<HTMLDivElement | null>(null);
  const [footerHeight, setFooterHeight] = useState(0);

  useEffect(() => {
    const el = footerRef.current;
    if (!el) {
      setFooterHeight(0);
      return;
    }
    // ResizeObserver may be absent in some test environments; fall back to a
    // one-shot measure so the layout still resolves.
    if (typeof ResizeObserver === 'undefined') {
      setFooterHeight(el.offsetHeight);
      return;
    }
    const ro = new ResizeObserver(() => setFooterHeight(el.offsetHeight));
    ro.observe(el);
    setFooterHeight(el.offsetHeight);
    return () => ro.disconnect();
  }, [footer]);

  return (
    <div className="relative flex h-full flex-col overflow-hidden bg-surface/70 dark:bg-black/40">
      {header}
      <div
        ref={scrollRef}
        data-testid="orch-chat-scroll"
        className="min-h-0 flex-1 overflow-y-auto"
        style={footer ? { paddingBottom: footerHeight } : undefined}>
        {children}
      </div>

      {/* Fade so messages dissolve into the background behind the composer. */}
      {footer ? (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-0 bottom-0 z-10 h-28 bg-gradient-to-t from-white via-white/90 to-transparent dark:from-black dark:via-black/90"
        />
      ) : null}

      {/* Floating, centered, width-capped composer footer over the fade. */}
      {footer ? (
        <div
          ref={footerRef}
          className="absolute inset-x-0 bottom-0 z-20 mx-auto w-full max-w-[48.75rem] px-4 pb-4 pt-6">
          {footer}
        </div>
      ) : null}
    </div>
  );
}

/**
 * The shared chat composer wired for the orchestration surfaces: attachments and
 * voice mode are off, and Enter sends (Shift+Enter inserts a newline). The host
 * owns the input value + the async send.
 */
function AgentComposer({
  value,
  setValue,
  onSend,
  isSending,
  placeholder,
}: {
  value: string;
  setValue: (value: string | ((prev: string) => string)) => void;
  onSend: (text?: string) => Promise<void>;
  isSending: boolean;
  placeholder: string;
}) {
  const textInputRef = useRef<HTMLTextAreaElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const isComposingTextRef = useRef(false);

  const handleInputKeyDown = useCallback(
    (event: KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.key === 'Enter' && !event.shiftKey && !isComposingTextRef.current) {
        event.preventDefault();
        void onSend();
      }
    },
    [onSend]
  );

  return (
    <ChatComposer
      inputValue={value}
      setInputValue={setValue}
      onSend={async text => {
        await onSend(text);
      }}
      textInputRef={textInputRef}
      fileInputRef={fileInputRef}
      composerInteractionBlocked={false}
      isSending={isSending}
      attachments={[]}
      onAttachFiles={async () => {}}
      onRemoveAttachment={() => {}}
      attachError={null}
      onSwitchToMicCloud={() => {}}
      handleInputKeyDown={handleInputKeyDown}
      inlineCompletionSuffix=""
      isComposingTextRef={isComposingTextRef}
      maxAttachments={0}
      allowedMimeTypes={[]}
      attachmentsEnabled={false}
      micEnabled={false}
      placeholder={placeholder}
    />
  );
}

/**
 * Peer-session subpage — takes over the whole agent pane (not a side drawer) so
 * the session's chat renders full-size, with a back link to return to the agent
 * chat. Replies go to the peer via the master send path.
 */
function SessionChatView({ session }: { session: SessionSummary }) {
  const { t } = useT();
  const { state, messages, refresh } = useSessionTranscript(session.sessionId);
  const { containerRef: scrollRef } = useStickToBottom(
    messages,
    session.sessionId,
    session.sessionId
  );
  const [body, setBody] = useState('');
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const send = useCallback(
    async (text?: string) => {
      const trimmed = (text ?? body).trim();
      if (!trimmed || sending) return;
      setSending(true);
      setSendError(null);
      debug('[orchestration:agent-chat] session reply: send session=%s', session.sessionId);
      try {
        await orchestrationClient.sendMasterMessage({
          body: trimmed,
          recipient: session.agentId,
          sessionId: session.sessionId,
        });
        if (!mountedRef.current) return;
        setBody('');
        void refresh();
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        debug(
          '[orchestration:agent-chat] session reply: failed session=%s %s',
          session.sessionId,
          message
        );
        if (mountedRef.current) setSendError(message);
      } finally {
        if (mountedRef.current) setSending(false);
      }
    },
    [body, sending, session.agentId, session.sessionId, refresh]
  );

  // A runtime tool-approval decision → reply "allow"/"deny" to the peer. Rethrows
  // on failure so SessionTranscript rolls the card back to buttons for a retry.
  const decide = useCallback(
    async (decision: 'allow' | 'deny'): Promise<void> => {
      setSendError(null);
      debug(
        '[orchestration:agent-chat] approval decision: send session=%s decision=%s',
        session.sessionId,
        decision
      );
      try {
        await orchestrationClient.sendMasterMessage({
          body: decision,
          recipient: session.agentId,
          sessionId: session.sessionId,
        });
        if (mountedRef.current) void refresh();
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        debug(
          '[orchestration:agent-chat] approval decision: failed session=%s %s',
          session.sessionId,
          message
        );
        if (mountedRef.current) setSendError(message);
        throw error;
      }
    },
    [session.agentId, session.sessionId, refresh]
  );

  const runtime = session.harnessType || session.source || null;
  const directory = session.workspace?.trim() || null;
  const runningOn = session.agentId?.trim() || null;

  return (
    <ChatPageScaffold
      scrollRef={scrollRef}
      header={
        // Agent metadata, centered to the same width-capped column as the chat.
        <div className="border-b border-line bg-surface/60 dark:bg-black/30">
          <div
            className="mx-auto w-full max-w-[48.75rem] px-5 py-3"
            data-testid="orch-session-header">
            <div className="flex items-center gap-2">
              <p className="text-sm font-semibold text-content">{sessionLabel(session)}</p>
              {session.status === 'waiting-approval' ? (
                <span className="flex-none rounded-full px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-300">
                  {t('orchPage.connections.status.needsYou')}
                </span>
              ) : null}
            </div>
            {/* Full values — they wrap/break only at the column's max width, never
                truncated at an arbitrary per-field cap. */}
            <dl className="mt-1.5 flex flex-wrap gap-x-5 gap-y-1 text-[11px]">
              {runtime ? (
                <div className="flex items-baseline gap-1">
                  <dt className="text-content-faint">{t('orchPage.session.runtime')}</dt>
                  <dd className="font-medium text-content-secondary">{runtime}</dd>
                </div>
              ) : null}
              {directory ? (
                <div className="flex items-baseline gap-1">
                  <dt className="text-content-faint">{t('orchPage.session.directory')}</dt>
                  <dd className="break-all font-mono font-medium text-content-secondary">
                    {directory}
                  </dd>
                </div>
              ) : null}
              {runningOn ? (
                <div className="flex items-baseline gap-1">
                  <dt className="text-content-faint">{t('orchPage.session.runningOn')}</dt>
                  <dd className="break-all font-mono font-medium text-content-secondary">
                    {runningOn}
                  </dd>
                </div>
              ) : null}
            </dl>
          </div>
        </div>
      }
      footer={
        <>
          {sendError ? (
            <p
              data-testid="orch-session-reply-error"
              className="mb-2 rounded-md bg-coral-50 px-2 py-1 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
              {t('tinyplaceOrchestration.composer.sendFailed')}: {sendError}
            </p>
          ) : null}
          <AgentComposer
            value={body}
            setValue={setBody}
            onSend={send}
            isSending={sending}
            placeholder={t('orchPage.connections.replyPlaceholder')}
          />
        </>
      }>
      <div className="mx-auto w-full max-w-[48.75rem] space-y-3 px-5 pt-4">
        {state.status === 'loading' ? (
          <p className="py-10 text-center text-sm text-content-muted">
            {t('tinyplaceOrchestration.loading')}
          </p>
        ) : state.status === 'error' ? (
          <p className="py-10 text-center text-sm text-coral-600 dark:text-coral-300">
            {t('tinyplaceOrchestration.failedToLoad')}: {state.message}
          </p>
        ) : messages.length === 0 ? (
          <p className="py-10 text-center text-sm text-content-faint">
            {t('tinyplaceOrchestration.noMessages')}
          </p>
        ) : (
          <SessionTranscript
            messages={messages}
            onDecide={(_message, decision) => decide(decision === 'deny' ? 'deny' : 'allow')}
          />
        )}
      </div>
    </ChatPageScaffold>
  );
}

export interface AgentChatPanelProps {
  /**
   * Controlled open peer-session id (the full-page session subpage). When
   * `onOpenSession` is provided the parent owns this (OrchestrationPage drives
   * it from the `?session=` query param + the sidebar's active sub-agents list);
   * otherwise the panel falls back to its own local state.
   */
  openSessionId?: string | null;
  onOpenSession?: (sessionId: string | null) => void;
}

export default function AgentChatPanel({
  openSessionId: controlledOpenSessionId,
  onOpenSession,
}: AgentChatPanelProps = {}) {
  const { t } = useT();
  const {
    sessionsState,
    messagesState,
    chats,
    selectedId,
    selected,
    status,
    masterError,
    selectChat,
    refresh,
    sendMessage,
  } = useOrchestrationChats(t);
  const contactSessions = useContactSessions();
  // Keep the transcript pinned to the newest message (and disengage when the
  // user scrolls up). Called before the `openSession` early return so hook order
  // stays stable; `selectedId` as thread + reset key snaps fresh on tab switch.
  const { containerRef: masterScrollRef } = useStickToBottom(
    selected?.messages ?? EMPTY_MESSAGES,
    selectedId,
    selectedId
  );

  const [composerBody, setComposerBody] = useState('');
  const [sending, setSending] = useState(false);
  const [runningReview, setRunningReview] = useState(false);
  // Controlled by the parent when `onOpenSession` is wired (OrchestrationPage
  // drives it from the URL + sidebar session list); local state otherwise.
  const [localOpenSessionId, setLocalOpenSessionId] = useState<string | null>(null);
  const openSessionId =
    controlledOpenSessionId !== undefined ? controlledOpenSessionId : localOpenSessionId;
  const setOpenSessionId = onOpenSession ?? setLocalOpenSessionId;
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Send the master composer's body via the orchestration send path.
  const sendComposer = useCallback(
    async (text?: string) => {
      const body = (text ?? composerBody).trim();
      if (!body || sending) return;
      setSending(true);
      const ok = await sendMessage(selected, body);
      if (!mountedRef.current) return;
      if (ok) setComposerBody('');
      setSending(false);
    },
    [composerBody, sending, sendMessage, selected]
  );

  const runSteeringReview = useCallback(async () => {
    debug('steering review: trigger');
    setRunningReview(true);
    try {
      // Steering review runs on the hosted brain now; nudge the device
      // subconscious (memory) so a manual tick still works locally.
      await subconsciousTrigger('all');
    } catch (err) {
      debug('steering review trigger failed: %o', err);
    } finally {
      setRunningReview(false);
    }
  }, []);

  const steeringText = status?.steering?.text?.trim() || null;
  const isMasterSelected = selectedId === MASTER_CHAT_KEY;
  const isSubconscious = selectedId === SUBCONSCIOUS_CHAT_KEY;

  const rail = chats.filter(c => c.id === MASTER_CHAT_KEY || c.id === SUBCONSCIOUS_CHAT_KEY);
  // Sessions the agent has engaged that are parked on an approval → "needs you".
  const pinged = contactSessions.sessions.filter(s => s.status === 'waiting-approval');
  const openSession = contactSessions.sessions.find(s => s.sessionId === openSessionId) ?? null;
  // Empty conscious thread → show the shared welcome hero (reuses the generic
  // chat's "new window" panel) instead of the bare "no messages" line.
  const showHero = isMasterSelected && (selected?.messages.length ?? 0) === 0;

  // A session is open → take over the whole pane with its full-page chat.
  // Returning to the master chat is done from the sidebar's Chat item (which
  // clears the `?session=` param), so no in-view back button is needed.
  if (openSession) {
    return <SessionChatView session={openSession} />;
  }

  const loading = sessionsState.status === 'loading' || messagesState.status === 'loading';
  const errored = sessionsState.status === 'error' || messagesState.status === 'error';

  // The Conscious/Subconscious toggle — sits in the composer footer slot where
  // the generic chat's super-context / quick-reasoning controls live.
  const modeToggle = (
    <div
      className="inline-flex h-7 items-center rounded-full border border-line bg-surface-subtle p-0.5"
      role="radiogroup"
      aria-label={t('orchPage.agent.modeLabel')}>
      {rail.map(chat => {
        const active = selectedId === chat.id;
        const label =
          chat.id === SUBCONSCIOUS_CHAT_KEY
            ? t('orchPage.agent.subconsciousTab')
            : t('orchPage.agent.consciousTab');
        return (
          <button
            key={chat.id}
            type="button"
            role="radio"
            aria-checked={active}
            data-testid={`orch-agent-tab-${chat.id}`}
            onClick={() => selectChat(chat.id)}
            className={`flex items-center gap-1.5 rounded-full px-3 py-0.5 text-xs font-medium transition-all ${
              active
                ? 'bg-surface text-content shadow-sm'
                : 'text-content-muted hover:text-content-secondary'
            }`}>
            {label}
            {chat.unread > 0 ? (
              <span className="inline-flex min-w-4 items-center justify-center rounded-full bg-primary-500 px-1 text-[10px] font-semibold text-content-inverted">
                {chat.unread}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );

  const showComposer = isMasterSelected && sessionsState.status === 'ok';

  return (
    // The main agent chat has no top header — just the conscious/subconscious
    // switching chip in the footer. When subconscious is active, the steering
    // directive + "Run review" ride alongside the chip (no header bar).
    <ChatPageScaffold
      scrollRef={masterScrollRef}
      footer={
        <div className="flex flex-col gap-2">
          {showComposer ? (
            <>
              {masterError ? (
                <p className="rounded-md bg-coral-50 px-2 py-1 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
                  {t('tinyplaceOrchestration.composer.sendFailed')}: {masterError}
                </p>
              ) : null}
              <AgentComposer
                value={composerBody}
                setValue={setComposerBody}
                onSend={sendComposer}
                isSending={sending}
                placeholder={t('tinyplaceOrchestration.composer.placeholder')}
              />
            </>
          ) : null}
          <div className="flex items-center justify-between gap-2">
            {modeToggle}
            {isSubconscious ? (
              <div className="flex min-w-0 items-center gap-2" data-testid="orch-agent-steering">
                {steeringText ? (
                  <span className="hidden min-w-0 truncate text-[11px] text-content-muted sm:inline">
                    {steeringText}
                  </span>
                ) : null}
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => void runSteeringReview()}
                  disabled={runningReview}>
                  {runningReview
                    ? t('tinyplaceOrchestration.steeringHeader.running')
                    : t('tinyplaceOrchestration.steeringHeader.runReview')}
                </Button>
              </div>
            ) : null}
          </div>
        </div>
      }>
      {loading ? (
        <div className="flex h-full items-center justify-center text-sm text-content-muted">
          {t('tinyplaceOrchestration.loading')}
        </div>
      ) : errored ? (
        <div className="flex h-full flex-col items-center justify-center gap-3 text-sm text-coral-600 dark:text-coral-300">
          <p>
            {t('tinyplaceOrchestration.failedToLoad')}:{' '}
            {sessionsState.status === 'error'
              ? sessionsState.message
              : messagesState.status === 'error'
                ? messagesState.message
                : ''}
          </p>
          <Button variant="secondary" size="sm" onClick={() => void refresh()}>
            {t('common.retry')}
          </Button>
        </div>
      ) : showHero && pinged.length === 0 ? (
        // Empty conscious thread: reuse the generic chat's welcome hero.
        <div className="mx-auto flex h-full w-full max-w-[48.75rem] px-5">
          <ChatNewWindowHero />
        </div>
      ) : (
        <div className="mx-auto w-full max-w-[48.75rem] space-y-3 px-5 pt-4">
          {selected?.messages.length ? (
            <SessionTranscript messages={selected.messages} />
          ) : showHero ? null : (
            <p className="py-10 text-center text-sm text-content-faint">
              {t('tinyplaceOrchestration.noMessages')}
            </p>
          )}

          {/* "Needs you" cards for sessions the agent engaged → open the session
              subpage (same as clicking it in the sidebar's active sub-agents). */}
          {pinged.map(session => (
            <div key={session.sessionId} className="flex justify-start">
              <button
                type="button"
                data-testid={`orch-agent-view-session-${session.sessionId}`}
                onClick={() => setOpenSessionId(session.sessionId)}
                className="flex w-full max-w-[85%] items-center gap-3 rounded-xl border border-primary-200 bg-primary-50 px-3 py-2.5 text-left transition hover:bg-primary-100/60 dark:border-primary-500/30 dark:bg-primary-900/20">
                <span className="flex h-8 w-8 flex-none items-center justify-center rounded-lg bg-primary-500/15 text-sm text-primary-600 dark:text-primary-300">
                  ⧉
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium text-content">
                    {sessionLabel(session)}
                  </span>
                  <span className="block truncate text-[11px] text-amber-600 dark:text-amber-300">
                    {t('orchPage.connections.status.needsYou')}
                  </span>
                </span>
                <span className="flex-none rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white">
                  {t('orchPage.agent.viewSession')}
                </span>
              </button>
            </div>
          ))}
        </div>
      )}
    </ChatPageScaffold>
  );
}
