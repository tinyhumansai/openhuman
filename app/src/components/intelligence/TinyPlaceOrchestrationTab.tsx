import debugFactory from 'debug';
import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { apiClient } from '../../agentworld/AgentWorldShell';
import { type PairingSnapshot, PaymentRequiredError } from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import {
  type AttentionAction,
  type AttentionQueue,
  orchestrationClient,
  type RelayInfo,
  type SelfIdentity,
} from '../../lib/orchestration/orchestrationClient';
import {
  type ChatWindow,
  MASTER_CHAT_KEY,
  useOrchestrationChats,
} from '../../lib/orchestration/useOrchestrationChats';
import { subconsciousTrigger } from '../../utils/tauriCommands/subconscious';
import Button from '../ui/Button';
import AttentionQueueView from './AttentionQueue';
import { ChatListButton } from './OrchestrationChatPrimitives';
import OrchestrationFocusPane from './OrchestrationFocusPane';
import {
  acceptedContactIds,
  chatTime,
  contactAddress,
  contactBadgeKey,
  extractHandle,
  pendingContactIds,
  truncate,
} from './orchestrationTabHelpers';
import RelayBadge from './RelayBadge';
import SelfIdentityCard from './SelfIdentityCard';

const debug = debugFactory('brain:tinyplace-orchestration');

// ── Pairing (unchanged data source: apiClient.orchestrationPairing.*) ─────────

type PairingState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'payment_required' }
  | { status: 'ok'; snapshot: PairingSnapshot };

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
  // Own tiny.place identity (discoverability) + the relay the core is on. Both
  // best-effort: a failed read leaves the card/badge hidden rather than erroring
  // the whole tab.
  const [selfIdentity, setSelfIdentity] = useState<SelfIdentity | null>(null);
  const [identityLoading, setIdentityLoading] = useState(true);
  const [relayInfo, setRelayInfo] = useState<RelayInfo | null>(null);
  // The aggregated "needs you" queue (approvals + blocked runs + unread). Read
  // independently of chats so a failure leaves the zone empty, never the tab.
  const [attentionQueue, setAttentionQueue] = useState<AttentionQueue | null>(null);
  const [attentionLoading, setAttentionLoading] = useState(true);
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

  const loadIdentity = useCallback(async () => {
    debug('[tinyplace-orchestration] identity load entry');
    // Identity and relay are independent reads: selfIdentity() builds the
    // tiny.place client from the wallet and can reject (locked/unconfigured
    // wallet), but relayInfo() only reads the configured base URL and must
    // stay visible regardless. Settle them separately so one failure never
    // hides the other. Neither failure may break the chat surface.
    const [identityResult, relayResult] = await Promise.allSettled([
      orchestrationClient.selfIdentity(),
      orchestrationClient.relayInfo(),
    ]);
    if (!mountedRef.current) return;
    if (identityResult.status === 'fulfilled') {
      debug(
        '[tinyplace-orchestration] identity load ok discoverable=%s',
        identityResult.value.discoverable
      );
      setSelfIdentity(identityResult.value);
    } else {
      const reason = identityResult.reason;
      const message = reason instanceof Error ? reason.message : String(reason);
      debug('[tinyplace-orchestration] identity load error %s', message);
    }
    if (relayResult.status === 'fulfilled') {
      debug('[tinyplace-orchestration] relay load ok network=%s', relayResult.value.network);
      setRelayInfo(relayResult.value);
    } else {
      const reason = relayResult.reason;
      const message = reason instanceof Error ? reason.message : String(reason);
      debug('[tinyplace-orchestration] relay load error %s', message);
    }
    setIdentityLoading(false);
  }, []);

  const loadAttention = useCallback(async () => {
    debug('[tinyplace-orchestration] attention load entry');
    try {
      const queue = await orchestrationClient.attention();
      if (!mountedRef.current) return;
      debug('[tinyplace-orchestration] attention load ok total=%d', queue.counts.total);
      setAttentionQueue(queue);
    } catch (error) {
      if (!mountedRef.current) return;
      const message = error instanceof Error ? error.message : String(error);
      debug('[tinyplace-orchestration] attention load error %s', message);
    } finally {
      if (mountedRef.current) setAttentionLoading(false);
    }
  }, []);

  // Route an attention item to its target. Only orchestration sessions have an
  // in-tab surface today; approvals/threads/runs live elsewhere (wired later).
  const handleAttentionAction = useCallback(
    (action: AttentionAction) => {
      debug('[tinyplace-orchestration] attention action type=%s', action.type);
      if (action.type === 'open-session') {
        selectChat(action.sessionId);
      }
    },
    [selectChat]
  );

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
    void loadIdentity();
    void loadAttention();
  }, [refresh, loadPairing, loadIdentity, loadAttention]);

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
    const handle = window.setTimeout(() => {
      void loadPairing();
      void loadIdentity();
      void loadAttention();
    }, 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
  }, [loadPairing, loadIdentity, loadAttention]);

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
              <div className="flex items-center gap-1.5">
                <h3 className="truncate text-sm font-semibold text-content">
                  {t('tinyplaceOrchestration.title')}
                </h3>
                <RelayBadge relay={relayInfo} />
              </div>
              <p className="mt-0.5 truncate text-[11px] text-content-muted">
                {t('tinyplaceOrchestration.subtitle')}
              </p>
            </div>
            <div className="flex flex-none items-center gap-1.5">
              <Button
                variant="secondary"
                size="sm"
                onClick={refreshAll}
                aria-label={t('tinyplaceOrchestration.refresh')}
                disabled={sessionsState.status === 'loading'}>
                {t('tinyplaceOrchestration.refresh')}
              </Button>
              {/* Launch shell — external instance spawn is wired in a later PR. */}
              <Button
                variant="primary"
                size="sm"
                data-testid="tinyplace-new-instance"
                disabled
                title={t('tinyplaceOrchestration.newInstanceSoon')}>
                {t('tinyplaceOrchestration.newInstance')}
              </Button>
            </div>
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

        <SelfIdentityCard identity={selfIdentity} loading={identityLoading} />

        <AttentionQueueView
          queue={attentionQueue}
          loading={attentionLoading}
          onAction={handleAttentionAction}
        />

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

      <OrchestrationFocusPane
        selected={selected}
        sessionsState={sessionsState}
        messagesState={messagesState}
        status={status}
        masterError={masterError}
        refresh={refresh}
        steeringText={steeringText}
        runningReview={runningReview}
        onRunSteeringReview={() => void runSteeringReview()}
        canCompose={canCompose}
        composerBody={composerBody}
        onComposerChange={setComposerBody}
        sending={sending}
        onSubmitComposer={submitComposer}
      />
    </div>
  );
}
