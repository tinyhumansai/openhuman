import debugFactory from 'debug';
import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { apiClient } from '../../agentworld/AgentWorldShell';
import {
  type ContactRequestsResponse,
  type ContactView,
  type InboxItem,
  type MessageEnvelope,
  type PairingSnapshot,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import Button from '../ui/Button';

const debug = debugFactory('brain:tinyplace-orchestration');

const MESSAGE_LIMIT = 100;
const INBOX_LIMIT = 40;
const ACTIVE_WINDOW_MS = 45 * 60 * 1000;

type ChatKind = 'master' | 'subconscious' | 'session';

interface ChatMessage {
  id: string;
  from: string;
  body: string;
  timestamp: string;
  encrypted: boolean;
}

interface ChatWindow {
  id: string;
  kind: ChatKind;
  title: string;
  subtitle: string;
  preview: string;
  lastTimestamp: string | null;
  unread: number;
  active: boolean;
  pinned: boolean;
  peerAgentId: string | null;
  messages: ChatMessage[];
}

interface TinyPlaceChatData {
  messages: MessageEnvelope[];
  inboxItems: InboxItem[];
  pairing: PairingSnapshot;
}

type LoadState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'payment_required' }
  | { status: 'ok'; data: TinyPlaceChatData };

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null;
}

function pickString(source: Record<string, unknown>, keys: string[]): string | null {
  for (const key of keys) {
    const value = asString(source[key]);
    if (value) return value;
  }
  return null;
}

function chatKindForEnvelope(envelope: MessageEnvelope): ChatKind {
  const type = (envelope.type ?? '').toLowerCase();
  if (type.includes('subconscious') || type.includes('internal')) return 'subconscious';
  if (type.includes('master') || type.includes('agent-human') || type.includes('human')) {
    return 'master';
  }
  return 'session';
}

function sessionIdForEnvelope(envelope: MessageEnvelope): string {
  return (
    pickString(envelope, ['sessionId', 'appSessionId', 'threadId', 'conversationId', 'runId']) ??
    `${envelope.from || 'unknown'}:${envelope.to || 'unknown'}`
  );
}

function isEncrypted(envelope: MessageEnvelope): boolean {
  const hint = (envelope.contentHint ?? '').toLowerCase();
  const type = (envelope.type ?? '').toLowerCase();
  return Boolean(envelope.signal) || hint.includes('encrypted') || type.includes('signal');
}

function displayBody(message: MessageEnvelope, encryptedText: string): string {
  if (isEncrypted(message)) return encryptedText;
  return message.body || message.contentHint || encryptedText;
}

function messageTime(message: Pick<ChatMessage, 'timestamp'>): number {
  const parsed = Date.parse(message.timestamp);
  return Number.isFinite(parsed) ? parsed : 0;
}

function sortMessages(messages: ChatMessage[]): ChatMessage[] {
  return messages.slice().sort((a, b) => messageTime(a) - messageTime(b));
}

function formatTime(timestamp: string | null): string {
  if (!timestamp) return '';
  const parsed = Date.parse(timestamp);
  if (!Number.isFinite(parsed)) return '';
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(new Date(parsed));
}

function truncate(text: string, length = 96): string {
  if (text.length <= length) return text;
  return `${text.slice(0, length - 1)}…`;
}

function isActive(lastTimestamp: string | null, unread: number): boolean {
  if (unread > 0) return true;
  if (!lastTimestamp) return false;
  const parsed = Date.parse(lastTimestamp);
  return Number.isFinite(parsed) && Date.now() - parsed < ACTIVE_WINDOW_MS;
}

function emptyPinnedChats(t: (key: string) => string): ChatWindow[] {
  return [
    {
      id: 'pinned:master',
      kind: 'master',
      title: t('tinyplaceOrchestration.master.title'),
      subtitle: t('tinyplaceOrchestration.master.subtitle'),
      preview: t('tinyplaceOrchestration.master.preview'),
      lastTimestamp: null,
      unread: 0,
      active: true,
      pinned: true,
      peerAgentId: null,
      messages: [],
    },
    {
      id: 'pinned:subconscious',
      kind: 'subconscious',
      title: t('tinyplaceOrchestration.subconscious.title'),
      subtitle: t('tinyplaceOrchestration.subconscious.subtitle'),
      preview: t('tinyplaceOrchestration.subconscious.preview'),
      lastTimestamp: null,
      unread: 0,
      active: true,
      pinned: true,
      peerAgentId: null,
      messages: [],
    },
  ];
}

function buildChats(data: TinyPlaceChatData, t: (key: string) => string): ChatWindow[] {
  const encryptedText = t('tinyplaceOrchestration.encryptedBody');
  const unknownSender = t('tinyplaceOrchestration.unknownSender');
  const pinned = emptyPinnedChats(t);
  const byId = new Map<string, ChatWindow>(pinned.map(chat => [chat.id, chat]));

  for (const envelope of data.messages) {
    const kind = chatKindForEnvelope(envelope);
    const id = kind === 'session' ? `session:${sessionIdForEnvelope(envelope)}` : `pinned:${kind}`;
    const message: ChatMessage = {
      id: envelope.id,
      from: envelope.from || unknownSender,
      body: displayBody(envelope, encryptedText),
      timestamp: envelope.timestamp,
      encrypted: isEncrypted(envelope),
    };
    const existing = byId.get(id);
    const title =
      kind === 'session'
        ? (pickString(envelope, ['sessionLabel', 'appName', 'threadTitle']) ??
          sessionIdForEnvelope(envelope))
        : (existing?.title ?? id);
    const subtitle =
      kind === 'session'
        ? (pickString(envelope, ['workspace', 'source', 'appSessionId']) ??
          t('tinyplaceOrchestration.session.subtitle'))
        : (existing?.subtitle ?? '');
    const nextMessages = sortMessages([...(existing?.messages ?? []), message]);
    const last = nextMessages[nextMessages.length - 1] ?? message;
    byId.set(id, {
      id,
      kind,
      title,
      subtitle,
      preview: truncate(last.body),
      lastTimestamp: last.timestamp,
      unread: existing?.unread ?? 0,
      active: true,
      pinned: kind !== 'session',
      peerAgentId: kind === 'session' ? envelope.from || envelope.to || null : null,
      messages: nextMessages,
    });
  }

  for (const item of data.inboxItems) {
    const sender = item.from ?? item.type ?? 'tiny.place';
    const id = `session:${sender}`;
    const message: ChatMessage = {
      id: item.itemId,
      from: sender,
      body: item.summary ?? item.subject,
      timestamp: item.timestamp,
      encrypted: false,
    };
    const existing = byId.get(id);
    const nextMessages = sortMessages([...(existing?.messages ?? []), message]);
    const last = nextMessages[nextMessages.length - 1] ?? message;
    const unread = (existing?.unread ?? 0) + (item.status === 'unread' ? 1 : 0);
    byId.set(id, {
      id,
      kind: 'session',
      title: sender,
      subtitle: item.type || t('tinyplaceOrchestration.session.subtitle'),
      preview: truncate(last.body),
      lastTimestamp: last.timestamp,
      unread,
      active: isActive(last.timestamp, unread),
      pinned: false,
      peerAgentId: sender,
      messages: nextMessages,
    });
  }

  return Array.from(byId.values()).map(chat => ({
    ...chat,
    active: chat.pinned ? true : isActive(chat.lastTimestamp, chat.unread),
  }));
}

function acceptedContactIds(contacts: ContactView[]): Set<string> {
  return new Set(
    contacts
      .filter(contact => contact.status === 'accepted')
      .map(contact => contact.agentId)
      .filter(Boolean)
  );
}

function pendingContactIds(requests: ContactRequestsResponse): Set<string> {
  return new Set(
    [...requests.incoming, ...requests.outgoing]
      .filter(contact => contact.status === 'pending')
      .map(contact => contact.agentId)
      .filter(Boolean)
  );
}

function contactBadgeKey(
  chat: ChatWindow,
  accepted: Set<string>,
  pending: Set<string>
): string | null {
  if (chat.pinned || !chat.peerAgentId) return null;
  if (accepted.has(chat.peerAgentId)) return 'tinyplaceOrchestration.pairing.linked';
  if (pending.has(chat.peerAgentId)) return 'tinyplaceOrchestration.pairing.pending';
  return 'tinyplaceOrchestration.pairing.unlinked';
}

function ChatListButton({
  chat,
  selected,
  onSelect,
  contactBadge,
}: {
  chat: ChatWindow;
  selected: boolean;
  onSelect: () => void;
  contactBadge?: string | null;
}) {
  const { t } = useT();
  return (
    <button
      type="button"
      data-testid={`tinyplace-chat-${chat.id}`}
      onClick={onSelect}
      className={`flex w-full items-start gap-3 border-b border-line-subtle px-3 py-3 text-left transition last:border-b-0 hover:bg-surface-hover ${
        selected ? 'bg-surface-muted' : ''
      }`}>
      <span className="mt-0.5 flex h-9 w-9 flex-none items-center justify-center rounded-lg border border-line bg-surface-strong text-xs font-semibold text-content-secondary">
        {chat.kind === 'subconscious' ? 'S' : chat.kind === 'master' ? 'M' : '#'}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center justify-between gap-2">
          <span className="truncate text-sm font-semibold text-content">{chat.title}</span>
          <span className="flex-none text-[10px] text-content-faint">
            {formatTime(chat.lastTimestamp)}
          </span>
        </span>
        <span className="mt-0.5 block truncate text-[11px] text-content-muted">
          {chat.subtitle}
        </span>
        <span className="mt-1 flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-xs text-content-faint">{chat.preview}</span>
          {chat.unread > 0 ? (
            <span className="flex-none rounded-full bg-ocean-500 px-1.5 py-0.5 text-[10px] font-semibold text-content-inverted">
              {chat.unread}
            </span>
          ) : null}
          {!chat.pinned ? (
            <span
              className={`flex-none rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
                chat.active
                  ? 'bg-sage-100 text-sage-700 dark:bg-sage-500/15 dark:text-sage-300'
                  : 'bg-surface-strong text-content-faint'
              }`}>
              {chat.active
                ? t('tinyplaceOrchestration.active')
                : t('tinyplaceOrchestration.inactive')}
            </span>
          ) : null}
          {contactBadge ? (
            <span className="flex-none rounded-full bg-surface-strong px-1.5 py-0.5 text-[10px] font-medium text-content-faint">
              {t(contactBadge)}
            </span>
          ) : null}
        </span>
      </span>
    </button>
  );
}

function MessageBubble({ message }: { message: ChatMessage }) {
  return (
    <div className="flex gap-2">
      <div className="mt-1.5 h-2 w-2 flex-none rounded-full bg-ocean-500" />
      <div className="min-w-0 rounded-lg border border-line bg-surface px-3 py-2 shadow-soft">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="text-xs font-semibold text-content-secondary">{message.from}</span>
          <span className="text-[10px] text-content-faint">{formatTime(message.timestamp)}</span>
        </div>
        <p
          className={`mt-1 whitespace-pre-wrap break-words text-sm ${
            message.encrypted ? 'text-content-muted' : 'text-content'
          }`}>
          {message.body}
        </p>
      </div>
    </div>
  );
}

export default function TinyPlaceOrchestrationTab() {
  const { t } = useT();
  const [state, setState] = useState<LoadState>({ status: 'loading' });
  const [selectedId, setSelectedId] = useState('pinned:master');
  const [linkAgentId, setLinkAgentId] = useState('');
  const [pairingAction, setPairingAction] = useState<string | null>(null);
  const [pairingError, setPairingError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const load = useCallback(async () => {
    debug('[tinyplace-orchestration] load entry');
    setState({ status: 'loading' });
    try {
      const [messages, inbox, pairing] = await Promise.all([
        apiClient.messages.list({ limit: MESSAGE_LIMIT }),
        apiClient.inbox.list({ limit: INBOX_LIMIT }),
        apiClient.orchestrationPairing.list(),
      ]);
      if (!mountedRef.current) return;
      debug(
        '[tinyplace-orchestration] load exit messages=%d inbox=%d contacts=%d incoming=%d outgoing=%d',
        messages.messages.length,
        inbox.items.length,
        pairing.contacts.contacts.length,
        pairing.requests.incoming.length,
        pairing.requests.outgoing.length
      );
      setState({
        status: 'ok',
        data: { messages: messages.messages, inboxItems: inbox.items, pairing },
      });
    } catch (error) {
      if (!mountedRef.current) return;
      if (error instanceof PaymentRequiredError) {
        debug('[tinyplace-orchestration] load payment_required');
        setState({ status: 'payment_required' });
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      debug('[tinyplace-orchestration] load error %s', message);
      setState({ status: 'error', message });
    }
  }, []);

  const runPairingAction = useCallback(
    async (actionId: string, action: () => Promise<unknown>) => {
      debug('[tinyplace-orchestration] pairing action entry id=%s', actionId);
      setPairingAction(actionId);
      setPairingError(null);
      try {
        await action();
        if (!mountedRef.current) return;
        debug('[tinyplace-orchestration] pairing action success id=%s', actionId);
        await load();
      } catch (error) {
        if (!mountedRef.current) return;
        const message = error instanceof Error ? error.message : String(error);
        debug('[tinyplace-orchestration] pairing action error id=%s %s', actionId, message);
        setPairingError(message);
      } finally {
        if (mountedRef.current) {
          setPairingAction(null);
        }
      }
    },
    [load]
  );

  const submitLink = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const agentId = linkAgentId.trim();
      if (!agentId) return;
      void runPairingAction(`request:${agentId}`, async () => {
        await apiClient.orchestrationPairing.linkSession(agentId);
        setLinkAgentId('');
      });
    },
    [linkAgentId, runPairingAction]
  );

  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => void load(), 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
  }, [load]);

  const chats = useMemo(
    () => (state.status === 'ok' ? buildChats(state.data, t) : emptyPinnedChats(t)),
    [state, t]
  );

  const resolvedSelectedId = chats.some(chat => chat.id === selectedId)
    ? selectedId
    : (chats[0]?.id ?? 'pinned:master');
  const selected = chats.find(chat => chat.id === resolvedSelectedId) ?? chats[0];
  const pinned = chats.filter(chat => chat.pinned);
  const sessions = chats
    .filter(chat => !chat.pinned)
    .sort(
      (a, b) =>
        Number(b.active) - Number(a.active) || messageTimeFromChat(b) - messageTimeFromChat(a)
    );
  const contactData = state.status === 'ok' ? state.data : null;
  const acceptedContacts = useMemo(
    () => acceptedContactIds(contactData?.pairing.contacts.contacts ?? []),
    [contactData?.pairing.contacts.contacts]
  );
  const pendingContacts = useMemo(
    () => pendingContactIds(contactData?.pairing.requests ?? { incoming: [], outgoing: [] }),
    [contactData?.pairing.requests]
  );
  const incomingRequests = contactData?.pairing.requests.incoming ?? [];
  const contactStats = contactData?.pairing.stats ?? null;

  return (
    <div className="flex min-h-[620px] overflow-hidden rounded-xl border border-line bg-surface shadow-soft">
      <aside className="flex w-80 flex-none flex-col border-r border-line bg-surface-muted/40">
        <div className="border-b border-line px-4 py-3">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <h3 className="truncate text-sm font-semibold text-content">
                {t('tinyplaceOrchestration.title')}
              </h3>
              <p className="mt-0.5 truncate text-[11px] text-content-muted">
                {t('tinyplaceOrchestration.subtitle')}
              </p>
            </div>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void load()}
              aria-label={t('tinyplaceOrchestration.refresh')}
              disabled={state.status === 'loading'}>
              {t('tinyplaceOrchestration.refresh')}
            </Button>
          </div>
        </div>

        <section className="border-b border-line px-4 py-3">
          <form className="space-y-2" onSubmit={submitLink}>
            <label
              htmlFor="tinyplace-session-agent-id"
              className="block text-[10px] font-semibold uppercase tracking-wide text-content-muted">
              {t('tinyplaceOrchestration.pairing.linkLabel')}
            </label>
            <div className="flex gap-2">
              <input
                id="tinyplace-session-agent-id"
                value={linkAgentId}
                onChange={event => setLinkAgentId(event.target.value)}
                placeholder={t('tinyplaceOrchestration.pairing.linkPlaceholder')}
                className="min-w-0 flex-1 rounded-md border border-line bg-surface px-2 py-1.5 text-xs text-content outline-none transition focus:border-ocean-500 focus:ring-2 focus:ring-ocean-500/20"
              />
              <Button
                type="submit"
                variant="secondary"
                size="sm"
                disabled={!linkAgentId.trim() || pairingAction !== null}>
                {t('tinyplaceOrchestration.pairing.linkAction')}
              </Button>
            </div>
          </form>

          <div className="mt-2 flex flex-wrap gap-1.5 text-[10px] text-content-faint">
            <span className="rounded-full bg-surface-strong px-2 py-0.5">
              {t('tinyplaceOrchestration.pairing.linked')}: {contactStats?.contactCount ?? 0}
            </span>
            <span className="rounded-full bg-surface-strong px-2 py-0.5">
              {t('tinyplaceOrchestration.pairing.incoming')}: {incomingRequests.length}
            </span>
            <span className="rounded-full bg-surface-strong px-2 py-0.5">
              {t('tinyplaceOrchestration.pairing.outgoing')}:{' '}
              {contactData?.pairing.requests.outgoing.length ?? 0}
            </span>
          </div>

          {pairingError ? (
            <p className="mt-2 rounded-md bg-coral-50 px-2 py-1 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
              {pairingError}
            </p>
          ) : null}

          {incomingRequests.length > 0 ? (
            <div className="mt-3 space-y-2">
              <h4 className="text-[10px] font-semibold uppercase tracking-wide text-content-muted">
                {t('tinyplaceOrchestration.pairing.requests')}
              </h4>
              {incomingRequests.map(request => (
                <div
                  key={request.agentId}
                  className="rounded-lg border border-line bg-surface px-2 py-2">
                  <div className="truncate text-xs font-medium text-content">{request.agentId}</div>
                  <div className="mt-2 flex gap-1.5">
                    <Button
                      variant="primary"
                      size="sm"
                      disabled={pairingAction !== null}
                      onClick={() =>
                        void runPairingAction(`accept:${request.agentId}`, () =>
                          apiClient.orchestrationPairing.acceptRequest(request.agentId)
                        )
                      }>
                      {t('tinyplaceOrchestration.pairing.accept')}
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={pairingAction !== null}
                      onClick={() =>
                        void runPairingAction(`remove:${request.agentId}`, () =>
                          apiClient.orchestrationPairing.declineRequest(request.agentId)
                        )
                      }>
                      {t('tinyplaceOrchestration.pairing.decline')}
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={pairingAction !== null}
                      onClick={() =>
                        void runPairingAction(`block:${request.agentId}`, () =>
                          apiClient.orchestrationPairing.blockRequest(request.agentId)
                        )
                      }>
                      {t('tinyplaceOrchestration.pairing.block')}
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          ) : null}
        </section>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <section>
            <h4 className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-muted">
              {t('tinyplaceOrchestration.pinned')}
            </h4>
            <div>
              {pinned.map(chat => (
                <ChatListButton
                  key={chat.id}
                  chat={chat}
                  selected={selected?.id === chat.id}
                  onSelect={() => {
                    debug('[tinyplace-orchestration] open pinned id=%s', chat.id);
                    setSelectedId(chat.id);
                  }}
                />
              ))}
            </div>
          </section>

          <section>
            <h4 className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-muted">
              {t('tinyplaceOrchestration.sessions')}
            </h4>
            {sessions.length === 0 ? (
              <div className="px-4 py-8 text-center text-sm text-content-faint">
                {t('tinyplaceOrchestration.noSessions')}
              </div>
            ) : (
              <div>
                {sessions.map(chat => (
                  <ChatListButton
                    key={chat.id}
                    chat={chat}
                    selected={selected?.id === chat.id}
                    contactBadge={contactBadgeKey(chat, acceptedContacts, pendingContacts)}
                    onSelect={() => {
                      debug('[tinyplace-orchestration] open session id=%s', chat.id);
                      setSelectedId(chat.id);
                    }}
                  />
                ))}
              </div>
            )}
          </section>
        </div>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col bg-surface">
        <div className="flex items-center justify-between gap-3 border-b border-line px-5 py-4">
          <div className="min-w-0">
            <h3 className="truncate text-base font-semibold text-content">{selected?.title}</h3>
            <p className="mt-0.5 truncate text-xs text-content-muted">{selected?.subtitle}</p>
          </div>
          {selected && !selected.pinned ? (
            <span
              className={`rounded-full px-2 py-1 text-xs font-medium ${
                selected.active
                  ? 'bg-sage-100 text-sage-700 dark:bg-sage-500/15 dark:text-sage-300'
                  : 'bg-surface-strong text-content-muted'
              }`}>
              {selected.active
                ? t('tinyplaceOrchestration.active')
                : t('tinyplaceOrchestration.inactive')}
            </span>
          ) : null}
        </div>

        {state.status === 'loading' ? (
          <div className="flex flex-1 items-center justify-center text-sm text-content-muted">
            {t('tinyplaceOrchestration.loading')}
          </div>
        ) : state.status === 'payment_required' ? (
          <div className="flex flex-1 items-center justify-center text-sm text-amber-600 dark:text-amber-300">
            {t('tinyplaceOrchestration.paymentRequired')}
          </div>
        ) : state.status === 'error' ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 text-sm text-coral-600 dark:text-coral-300">
            <p>
              {t('tinyplaceOrchestration.failedToLoad')}: {state.message}
            </p>
            <Button variant="secondary" size="sm" onClick={() => void load()}>
              {t('common.retry')}
            </Button>
          </div>
        ) : selected?.messages.length ? (
          <div className="min-h-0 flex-1 overflow-y-auto bg-surface-muted/20 p-5">
            <div className="space-y-3" data-testid="tinyplace-chat-messages">
              {selected.messages.map(message => (
                <MessageBubble key={message.id} message={message} />
              ))}
            </div>
          </div>
        ) : (
          <div className="flex flex-1 items-center justify-center px-6 text-center text-sm text-content-faint">
            {t('tinyplaceOrchestration.noMessages')}
          </div>
        )}
      </main>
    </div>
  );
}

function messageTimeFromChat(chat: ChatWindow): number {
  if (!chat.lastTimestamp) return 0;
  const parsed = Date.parse(chat.lastTimestamp);
  return Number.isFinite(parsed) ? parsed : 0;
}
