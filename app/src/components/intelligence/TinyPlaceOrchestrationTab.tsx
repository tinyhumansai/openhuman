import debugFactory from 'debug';
import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { apiClient } from '../../agentworld/AgentWorldShell';
import {
  type ContactRequestsResponse,
  type ContactView,
  type PairingSnapshot,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import {
  type ChatMessage,
  type ChatWindow,
  MASTER_CHAT_KEY,
  useOrchestrationChats,
} from '../../lib/orchestration/useOrchestrationChats';
import { subconsciousTrigger } from '../../utils/tauriCommands/subconscious';
import Button from '../ui/Button';

const debug = debugFactory('brain:tinyplace-orchestration');

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

function acceptedContactIds(contacts: ContactView[]): Set<string> {
  return new Set(
    contacts
      .filter(contact => contact.status === 'accepted')
      .map(contactAddress)
      .filter(Boolean)
  );
}

function pendingContactIds(requests: ContactRequestsResponse): Set<string> {
  return new Set(
    [...requests.incoming, ...requests.outgoing]
      .filter(contact => contact.status === 'pending')
      .map(contactAddress)
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
          {chat.kind === 'subconscious'
            ? t('tinyplaceOrchestration.subconsciousBadge')
            : chat.subtitle}
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

// ── Pairing (unchanged data source: apiClient.orchestrationPairing.*) ─────────

type PairingState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'payment_required' }
  | { status: 'ok'; snapshot: PairingSnapshot };

/** Best-effort `@handle` for a tiny.place agent id (cryptoId) from a directory
 * reverse lookup — the registered username if any, else null. The raw address
 * is always shown; the handle is additive. */
function extractHandle(res: {
  agents?: Array<{ username?: string }>;
  identities?: unknown[];
}): string | null {
  const fromAgent = res.agents?.find(a => a.username)?.username;
  const fromIdentity = (res.identities as Array<{ username?: string }> | undefined)?.find(
    identity => identity?.username
  )?.username;
  const username = fromAgent ?? fromIdentity;
  return username ? username.replace(/^@+/, '') : null;
}

// The counterpart agent address for a contact view (request or accepted
// contact). The relay's `/contacts` and `/contacts/requests` payloads do not
// always populate the top-level `agentId`, so fall back to the underlying
// contact record: when we are the addressee the counterpart is the
// `requester`, otherwise it is the `addressee`.
function contactAddress(view: ContactView): string {
  if (view.agentId) return view.agentId;
  const contact = view.contact;
  if (!contact) return '';
  return view.direction === 'outgoing' ? (contact.addressee ?? '') : (contact.requester ?? '');
}

export default function TinyPlaceOrchestrationTab() {
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
    createSession,
  } = useOrchestrationChats(t);

  const [pairingState, setPairingState] = useState<PairingState>({ status: 'loading' });
  const [linkAgentId, setLinkAgentId] = useState('');
  const [pairingAction, setPairingAction] = useState<string | null>(null);
  const [pairingError, setPairingError] = useState<string | null>(null);
  const [composerBody, setComposerBody] = useState('');
  const [sending, setSending] = useState(false);
  // Resolved `@handle`s for agent ids seen in the pairing UI (address always shown).
  const [agentHandles, setAgentHandles] = useState<Record<string, string | null>>({});
  // Which contact rows are expanded to reveal their nested sessions.
  const [expandedContacts, setExpandedContacts] = useState<Record<string, boolean>>({});
  const [creatingSession, setCreatingSession] = useState<string | null>(null);
  const mountedRef = useRef(true);

  const toggleContact = useCallback((address: string) => {
    setExpandedContacts(prev => ({ ...prev, [address]: !prev[address] }));
  }, []);

  const handleCreateSession = useCallback(
    (address: string) => {
      if (!address || creatingSession) return;
      setCreatingSession(address);
      setExpandedContacts(prev => ({ ...prev, [address]: true }));
      void createSession(address).finally(() => {
        if (mountedRef.current) setCreatingSession(null);
      });
    },
    [createSession, creatingSession]
  );

  const loadPairing = useCallback(async () => {
    debug('[tinyplace-orchestration] pairing load entry');
    setPairingState({ status: 'loading' });
    try {
      const snapshot = await apiClient.orchestrationPairing.list();
      if (!mountedRef.current) return;
      debug(
        '[tinyplace-orchestration] pairing load exit contacts=%d incoming=%d outgoing=%d',
        snapshot.contacts.contacts.length,
        snapshot.requests.incoming.length,
        snapshot.requests.outgoing.length
      );
      setPairingState({ status: 'ok', snapshot });
    } catch (error) {
      if (!mountedRef.current) return;
      if (error instanceof PaymentRequiredError) {
        debug('[tinyplace-orchestration] pairing payment_required');
        setPairingState({ status: 'payment_required' });
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      debug('[tinyplace-orchestration] pairing load error %s', message);
      setPairingState({ status: 'error', message });
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
        await loadPairing();
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
    [loadPairing]
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

  const refreshAll = useCallback(() => {
    void refresh();
    void loadPairing();
  }, [refresh, loadPairing]);

  const submitComposer = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const body = composerBody.trim();
      if (!body || sending) return;
      setSending(true);
      void sendMessage(selected, body).then(ok => {
        if (!mountedRef.current) return;
        if (ok) setComposerBody('');
        setSending(false);
      });
    },
    [composerBody, sending, sendMessage, selected]
  );

  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => void loadPairing(), 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
  }, [loadPairing]);

  const pinned = chats.filter(chat => chat.pinned);
  const sessions = chats
    .filter(chat => !chat.pinned)
    .sort((a, b) => Number(b.active) - Number(a.active) || chatTime(b) - chatTime(a));

  const pairingSnapshot = pairingState.status === 'ok' ? pairingState.snapshot : null;
  const acceptedContacts = useMemo(
    () => acceptedContactIds(pairingSnapshot?.contacts.contacts ?? []),
    [pairingSnapshot?.contacts.contacts]
  );
  const pendingContacts = useMemo(
    () => pendingContactIds(pairingSnapshot?.requests ?? { incoming: [], outgoing: [] }),
    [pairingSnapshot?.requests]
  );
  const incomingRequests = pairingSnapshot?.requests.incoming ?? [];
  const acceptedContactList = useMemo(
    () =>
      (pairingSnapshot?.contacts.contacts ?? []).filter(contact => contact.status === 'accepted'),
    [pairingSnapshot?.contacts.contacts]
  );
  const contactStats = pairingSnapshot?.stats ?? null;

  // Group session chats under their peer contact for the nested sidebar tree.
  const sessionsByContact = new Map<string, ChatWindow[]>();
  for (const chat of sessions) {
    if (!chat.peerAgentId) continue;
    const list = sessionsByContact.get(chat.peerAgentId) ?? [];
    list.push(chat);
    sessionsByContact.set(chat.peerAgentId, list);
  }
  const contactAddressSet = new Set(acceptedContactList.map(contactAddress).filter(Boolean));
  // Sessions whose peer is not a known accepted contact still need a home.
  const ungroupedSessions = sessions.filter(
    chat => !chat.peerAgentId || !contactAddressSet.has(chat.peerAgentId)
  );

  // Resolve @handles for the agent ids seen in the pairing UI (incoming
  // requests + accepted contacts) via the directory reverse lookup
  // (best-effort; the raw address is always rendered).
  const directoryIdsKey = [...incomingRequests, ...acceptedContactList]
    .map(contactAddress)
    .filter(Boolean)
    .join(',');
  useEffect(() => {
    const ids = directoryIdsKey ? Array.from(new Set(directoryIdsKey.split(','))) : [];
    if (ids.length === 0) return;
    let cancelled = false;
    void Promise.all(
      ids.map(async id => {
        try {
          return [id, extractHandle(await apiClient.directory.reverse(id))] as const;
        } catch {
          return [id, null] as const;
        }
      })
    ).then(entries => {
      if (cancelled) return;
      setAgentHandles(prev => {
        const next = { ...prev };
        for (const [id, handle] of entries) {
          if (!(id in next)) next[id] = handle;
        }
        return next;
      });
    });
    return () => {
      cancelled = true;
    };
  }, [directoryIdsKey]);

  const steeringText = status?.steering?.text?.trim() || null;
  const [runningReview, setRunningReview] = useState(false);
  const runSteeringReview = useCallback(async () => {
    setRunningReview(true);
    try {
      await subconsciousTrigger('tinyplace');
    } catch (err) {
      debug('steering review trigger failed: %o', err);
    } finally {
      setRunningReview(false);
    }
  }, []);
  const isMasterSelected = selected?.id === MASTER_CHAT_KEY;
  // The composer is available for the Master chat and for any per-contact
  // session (session sends thread under that session id).
  const canCompose = isMasterSelected || selected?.kind === 'session';

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
              onClick={refreshAll}
              aria-label={t('tinyplaceOrchestration.refresh')}
              disabled={sessionsState.status === 'loading'}>
              {t('tinyplaceOrchestration.refresh')}
            </Button>
          </div>
          {steeringText ? (
            <div
              data-testid="tinyplace-steering-chip"
              className="mt-2 flex items-start gap-1.5 rounded-md bg-amber-50 px-2 py-1 text-[11px] text-amber-700 dark:bg-amber-500/10 dark:text-amber-300">
              <span className="flex-none font-semibold uppercase tracking-wide">
                {t('tinyplaceOrchestration.steering.label')}
              </span>
              <span className="min-w-0 flex-1 truncate">{truncate(steeringText, 72)}</span>
            </div>
          ) : null}
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
              {pairingSnapshot?.requests.outgoing.length ?? 0}
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
              {incomingRequests.map((request, index) => {
                const address = contactAddress(request);
                const handle = address ? agentHandles[address] : null;
                return (
                  <div
                    key={address || `request-${index}`}
                    className="rounded-lg border border-line bg-surface px-2 py-2">
                    {handle ? (
                      <div className="truncate text-xs font-medium text-content">@{handle}</div>
                    ) : null}
                    <div className="truncate font-mono text-[11px] text-content-muted">
                      {address}
                    </div>
                    <div className="mt-2 flex gap-1.5">
                      <Button
                        variant="primary"
                        size="sm"
                        disabled={pairingAction !== null || !address}
                        onClick={() =>
                          void runPairingAction(`accept:${address}`, () =>
                            apiClient.orchestrationPairing.acceptRequest(address)
                          )
                        }>
                        {t('tinyplaceOrchestration.pairing.accept')}
                      </Button>
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={pairingAction !== null || !address}
                        onClick={() =>
                          void runPairingAction(`remove:${address}`, () =>
                            apiClient.orchestrationPairing.declineRequest(address)
                          )
                        }>
                        {t('tinyplaceOrchestration.pairing.decline')}
                      </Button>
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={pairingAction !== null || !address}
                        onClick={() =>
                          void runPairingAction(`block:${address}`, () =>
                            apiClient.orchestrationPairing.blockRequest(address)
                          )
                        }>
                        {t('tinyplaceOrchestration.pairing.block')}
                      </Button>
                    </div>
                  </div>
                );
              })}
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
                  selected={selectedId === chat.id}
                  onSelect={() => {
                    debug('[tinyplace-orchestration] open pinned id=%s', chat.id);
                    selectChat(chat.id);
                  }}
                />
              ))}
            </div>
          </section>

          <section>
            <h4 className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-muted">
              {t('tinyplaceOrchestration.contacts')}
            </h4>
            {acceptedContactList.length === 0 ? (
              <div className="px-4 py-8 text-center text-sm text-content-faint">
                {t('tinyplaceOrchestration.noContacts')}
              </div>
            ) : (
              <div className="space-y-1 px-2 pb-2">
                {acceptedContactList.map((contact, index) => {
                  const address = contactAddress(contact);
                  const handle = address ? agentHandles[address] : null;
                  const isOpen = !!expandedContacts[address];
                  const contactSessions = address ? (sessionsByContact.get(address) ?? []) : [];
                  return (
                    <div
                      key={address || `contact-${index}`}
                      className="overflow-hidden rounded-lg border border-line bg-surface">
                      <button
                        type="button"
                        data-testid={`tinyplace-contact-${address}`}
                        aria-expanded={isOpen}
                        onClick={() => toggleContact(address)}
                        className="flex w-full items-center gap-2 px-2 py-2 text-left transition hover:bg-surface-hover">
                        <span className="flex-none text-[10px] text-content-muted">
                          {isOpen ? '▾' : '▸'}
                        </span>
                        <span className="min-w-0 flex-1">
                          {handle ? (
                            <span className="block truncate text-xs font-medium text-content">
                              @{handle}
                            </span>
                          ) : null}
                          <span className="block truncate font-mono text-[11px] text-content-muted">
                            {address}
                          </span>
                        </span>
                        {contactSessions.length > 0 ? (
                          <span className="flex-none rounded-full bg-surface-strong px-1.5 py-0.5 text-[10px] font-medium text-content-faint">
                            {contactSessions.length}
                          </span>
                        ) : null}
                      </button>
                      {isOpen ? (
                        <div className="border-t border-line-subtle">
                          {contactSessions.map(chat => (
                            <ChatListButton
                              key={chat.id}
                              chat={chat}
                              selected={selectedId === chat.id}
                              contactBadge={contactBadgeKey(
                                chat,
                                acceptedContacts,
                                pendingContacts
                              )}
                              onSelect={() => {
                                debug('[tinyplace-orchestration] open session id=%s', chat.id);
                                selectChat(chat.id);
                              }}
                            />
                          ))}
                          <button
                            type="button"
                            data-testid={`tinyplace-new-session-${address}`}
                            disabled={!address || creatingSession === address}
                            onClick={() => handleCreateSession(address)}
                            className="flex w-full items-center gap-1 px-3 py-2 text-left text-[11px] font-medium text-ocean-500 transition hover:bg-surface-hover disabled:opacity-50">
                            + {t('tinyplaceOrchestration.newSession')}
                          </button>
                        </div>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            )}
          </section>

          {ungroupedSessions.length > 0 ? (
            <section>
              <h4 className="px-3 pb-1 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-muted">
                {t('tinyplaceOrchestration.otherSessions')}
              </h4>
              <div>
                {ungroupedSessions.map(chat => (
                  <ChatListButton
                    key={chat.id}
                    chat={chat}
                    selected={selectedId === chat.id}
                    contactBadge={contactBadgeKey(chat, acceptedContacts, pendingContacts)}
                    onSelect={() => {
                      debug('[tinyplace-orchestration] open session id=%s', chat.id);
                      selectChat(chat.id);
                    }}
                  />
                ))}
              </div>
            </section>
          ) : null}
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

        {/* Steering status header — the tinyplace subconscious instance's output. */}
        {selected?.kind === 'subconscious' ? (
          <div
            data-testid="tinyplace-steering-header"
            className="flex items-center justify-between gap-3 border-b border-line bg-amber-50/40 px-5 py-3 dark:bg-amber-500/5">
            <div className="min-w-0">
              <p className="text-xs font-medium text-content">
                {steeringText
                  ? t('tinyplaceOrchestration.steeringHeader.current')
                  : t('tinyplaceOrchestration.steeringHeader.none')}
              </p>
              {steeringText ? (
                <p className="mt-0.5 truncate text-xs text-content-muted">{steeringText}</p>
              ) : null}
              <p className="mt-0.5 text-[11px] text-content-faint">
                {status?.steering
                  ? t('tinyplaceOrchestration.steeringHeader.expires').replace(
                      '{n}',
                      String(status.steering.expiresAfterCycles)
                    )
                  : ''}
                {status?.lastTickAt
                  ? `${status?.steering ? ' · ' : ''}${t(
                      'tinyplaceOrchestration.steeringHeader.lastReview'
                    )}: ${new Date(status.lastTickAt * 1000).toLocaleTimeString()}`
                  : ''}
              </p>
            </div>
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

        {sessionsState.status === 'loading' ? (
          <div className="flex flex-1 items-center justify-center text-sm text-content-muted">
            {t('tinyplaceOrchestration.loading')}
          </div>
        ) : sessionsState.status === 'payment_required' ? (
          <div className="flex flex-1 items-center justify-center text-sm text-amber-600 dark:text-amber-300">
            {t('tinyplaceOrchestration.paymentRequired')}
          </div>
        ) : sessionsState.status === 'error' ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 text-sm text-coral-600 dark:text-coral-300">
            <p>
              {t('tinyplaceOrchestration.failedToLoad')}: {sessionsState.message}
            </p>
            <Button variant="secondary" size="sm" onClick={() => void refresh()}>
              {t('common.retry')}
            </Button>
          </div>
        ) : messagesState.status === 'loading' ? (
          <div className="flex flex-1 items-center justify-center text-sm text-content-muted">
            {t('tinyplaceOrchestration.loading')}
          </div>
        ) : messagesState.status === 'error' ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 text-sm text-coral-600 dark:text-coral-300">
            <p>
              {t('tinyplaceOrchestration.failedToLoad')}: {messagesState.message}
            </p>
            <Button variant="secondary" size="sm" onClick={() => void refresh()}>
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

        {canCompose && sessionsState.status === 'ok' ? (
          <form
            className="flex flex-col gap-2 border-t border-line px-5 py-3"
            onSubmit={submitComposer}>
            {masterError ? (
              <p className="rounded-md bg-coral-50 px-2 py-1 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
                {t('tinyplaceOrchestration.composer.sendFailed')}: {masterError}
              </p>
            ) : null}
            <div className="flex gap-2">
              <input
                data-testid="tinyplace-master-composer-input"
                value={composerBody}
                onChange={event => setComposerBody(event.target.value)}
                placeholder={t('tinyplaceOrchestration.composer.placeholder')}
                className="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-2 text-sm text-content outline-none transition focus:border-ocean-500 focus:ring-2 focus:ring-ocean-500/20"
              />
              <Button
                type="submit"
                variant="primary"
                size="sm"
                data-testid="tinyplace-master-composer-send"
                disabled={!composerBody.trim() || sending}>
                {t('tinyplaceOrchestration.composer.send')}
              </Button>
            </div>
          </form>
        ) : null}
      </main>
    </div>
  );
}

function chatTime(chat: ChatWindow): number {
  if (!chat.lastTimestamp) return 0;
  const parsed = Date.parse(chat.lastTimestamp);
  return Number.isFinite(parsed) ? parsed : 0;
}
