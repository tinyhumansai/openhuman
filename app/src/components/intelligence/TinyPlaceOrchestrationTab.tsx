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
import OrchestrationFocusPane from './OrchestrationFocusPane';
import OrchestrationSidebar from './OrchestrationSidebar';
import {
  acceptedContactIds,
  chatTime,
  contactAddress,
  extractHandle,
  pendingContactIds,
} from './orchestrationTabHelpers';

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
  // "Make discoverable" remediation: publishes the directory card + Signal key
  // when the identity card reports the agent is un-messageable.
  const [publishingIdentity, setPublishingIdentity] = useState(false);
  const [publishIdentityError, setPublishIdentityError] = useState<string | null>(null);
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

  // Publish (or refresh) the directory card + Signal key, then adopt the fresh
  // identity the RPC echoes back so the card flips to "discoverable" without a
  // separate reload. A failure surfaces on the card, not the whole tab.
  const publishIdentity = useCallback(async () => {
    debug('[tinyplace-orchestration] publish identity entry');
    setPublishingIdentity(true);
    setPublishIdentityError(null);
    try {
      const identity = await orchestrationClient.publishIdentity();
      if (!mountedRef.current) return;
      debug('[tinyplace-orchestration] publish identity ok discoverable=%s', identity.discoverable);
      setSelfIdentity(identity);
    } catch (error) {
      if (!mountedRef.current) return;
      const message = error instanceof Error ? error.message : String(error);
      debug('[tinyplace-orchestration] publish identity error %s', message);
      setPublishIdentityError(message);
    } finally {
      if (mountedRef.current) setPublishingIdentity(false);
    }
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
      <OrchestrationSidebar
        relayInfo={relayInfo}
        onRefreshAll={refreshAll}
        refreshDisabled={sessionsState.status === 'loading'}
        steeringText={steeringText}
        selfIdentity={selfIdentity}
        identityLoading={identityLoading}
        onPublishIdentity={publishIdentity}
        publishingIdentity={publishingIdentity}
        publishIdentityError={publishIdentityError}
        attentionQueue={attentionQueue}
        attentionLoading={attentionLoading}
        onAttentionAction={handleAttentionAction}
        linkAgentId={linkAgentId}
        onLinkAgentIdChange={setLinkAgentId}
        onSubmitLink={submitLink}
        pairingAction={pairingAction}
        contactStats={contactStats}
        incomingRequests={incomingRequests}
        outgoingCount={pairingSnapshot?.requests.outgoing.length ?? 0}
        pairingError={pairingError}
        agentHandles={agentHandles}
        runPairingAction={runPairingAction}
        pinned={pinned}
        selectedId={selectedId}
        onSelectChat={selectChat}
        acceptedContactList={acceptedContactList}
        expandedContacts={expandedContacts}
        onToggleContact={toggleContact}
        sessionsByContact={sessionsByContact}
        creatingSession={creatingSession}
        onCreateSession={handleCreateSession}
        acceptedContacts={acceptedContacts}
        pendingContacts={pendingContacts}
        ungroupedSessions={ungroupedSessions}
      />

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
