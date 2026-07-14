/**
 * ConnectionsPanel — the agent's connections **and their sessions**.
 *
 * A list of accepted tiny.place contacts where each contact expands to reveal
 * your sessions with it (status · last activity · message count). Opening a
 * session shows its conversation history in place — rendered in the app's normal
 * chat-window style via {@link SessionTranscript} — with an inline reply
 * composer. Pending requests + a summary sit on top. Linking new agents still
 * lives in the sibling DiscoverPanel.
 *
 * Renders inside the same single `max-w-3xl` column OrchestrationPage gives it.
 */
import debugFactory from 'debug';
import { type FormEvent, useCallback, useEffect, useMemo, useState } from 'react';

import { apiClient } from '../../agentworld/AgentWorldShell';
import { useT } from '../../lib/i18n/I18nContext';
import {
  type InstanceStatus,
  orchestrationClient,
  type SessionSummary,
} from '../../lib/orchestration/orchestrationClient';
import {
  useContactSessions,
  useSessionTranscript,
} from '../../lib/orchestration/useOrchestrationSessions';
import { usePairing } from '../../lib/orchestration/usePairing';
import { contactAddress, extractHandle } from '../intelligence/orchestrationTabHelpers';
import Button from '../ui/Button';
import { SectionCard, StatTile } from './primitives';
import SessionTranscript from './SessionTranscript';

const debug = debugFactory('orchestration:connections');

type Translate = (key: string, fallback?: string) => string;

interface StatusMeta {
  label: string;
  dot: string;
  tone: string;
}

function statusMeta(status: InstanceStatus, t: Translate): StatusMeta {
  switch (status) {
    case 'waiting-approval':
      return {
        label: t('orchPage.connections.status.needsYou'),
        dot: 'bg-amber-500',
        tone: 'text-amber-700 dark:text-amber-300',
      };
    case 'running':
      return {
        label: t('orchPage.connections.status.running'),
        dot: 'bg-sage-500',
        tone: 'text-sage-700 dark:text-sage-300',
      };
    case 'errored':
      return {
        label: t('orchPage.connections.status.error'),
        dot: 'bg-coral-500',
        tone: 'text-coral-700 dark:text-coral-300',
      };
    case 'idle':
      return {
        label: t('orchPage.connections.status.idle'),
        dot: 'bg-content-faint',
        tone: 'text-content-muted',
      };
    default:
      return {
        label: t('orchPage.connections.status.done'),
        dot: 'bg-content-faint',
        tone: 'text-content-muted',
      };
  }
}

function shortAddress(address: string): string {
  return address.length <= 14 ? address : `${address.slice(0, 6)}…${address.slice(-4)}`;
}

// ── Session view (in place) ──────────────────────────────────────────────────
function SessionView({
  session,
  contactAddr,
  handle,
  onBack,
}: {
  session: SessionSummary;
  contactAddr: string;
  handle: string | null;
  onBack: () => void;
}) {
  const { t } = useT();
  const { state, messages, refresh } = useSessionTranscript(session.sessionId);
  const [body, setBody] = useState('');
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const label = session.label?.trim() || session.sessionId;
  const meta = statusMeta(session.status, t);

  const submit = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const trimmed = body.trim();
      if (!trimmed || sending) return;
      setSending(true);
      setSendError(null);
      debug('[orchestration:connections] session reply: send session=%s', session.sessionId);
      void orchestrationClient
        .sendMasterMessage({ body: trimmed, recipient: contactAddr, sessionId: session.sessionId })
        .then(() => {
          setBody('');
          void refresh();
        })
        .catch((error: unknown) => {
          const message = error instanceof Error ? error.message : String(error);
          debug(
            '[orchestration:connections] session reply: failed session=%s %s',
            session.sessionId,
            message
          );
          setSendError(message);
        })
        .finally(() => setSending(false));
    },
    [body, sending, contactAddr, session.sessionId, refresh]
  );

  // Runtime tool-approval decision → reply "allow"/"deny" to the runtime peer (the
  // same send path as a typed reply). Returns the promise (rethrows on failure) so
  // SessionTranscript rolls the card back to buttons for a retry.
  const decide = useCallback(
    (decision: 'allow' | 'deny'): Promise<void> => {
      setSendError(null);
      debug(
        '[orchestration:connections] approval decision: send session=%s decision=%s',
        session.sessionId,
        decision
      );
      return orchestrationClient
        .sendMasterMessage({ body: decision, recipient: contactAddr, sessionId: session.sessionId })
        .then(() => {
          void refresh();
        })
        .catch((error: unknown) => {
          const message = error instanceof Error ? error.message : String(error);
          debug(
            '[orchestration:connections] approval decision: failed session=%s %s',
            session.sessionId,
            message
          );
          setSendError(message);
          throw error;
        });
    },
    [contactAddr, session.sessionId, refresh]
  );

  return (
    <div className="space-y-3" data-testid="orch-session-view">
      <button
        type="button"
        onClick={onBack}
        data-testid="orch-session-back"
        className="flex items-center gap-1.5 text-xs font-medium text-primary-600 transition hover:text-primary-700 dark:text-primary-300">
        ← {t('orchPage.connections.back')} / {handle ? `@${handle}` : shortAddress(contactAddr)}
      </button>
      <SectionCard
        title={
          <span className="flex items-center gap-2">
            {label}
            <span className={`rounded-full px-1.5 py-0.5 text-[10px] font-medium ${meta.tone}`}>
              {meta.label}
            </span>
          </span>
        }
        description={
          session.messageCount
            ? t('orchPage.connections.messageCount').replace('{n}', String(session.messageCount))
            : undefined
        }>
        {state.status === 'loading' ? (
          <p className="py-6 text-center text-sm text-content-muted">
            {t('tinyplaceOrchestration.loading')}
          </p>
        ) : state.status === 'error' ? (
          <p className="py-6 text-center text-sm text-coral-600 dark:text-coral-300">
            {t('tinyplaceOrchestration.failedToLoad')}: {state.message}
          </p>
        ) : messages.length === 0 ? (
          <p className="py-6 text-center text-sm text-content-faint">
            {t('tinyplaceOrchestration.noMessages')}
          </p>
        ) : (
          <SessionTranscript
            messages={messages}
            onDecide={(_message, decision) => decide(decision === 'deny' ? 'deny' : 'allow')}
          />
        )}
        {sendError ? (
          <p
            data-testid="orch-session-reply-error"
            className="mt-3 rounded-md bg-coral-50 px-2 py-1 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
            {t('tinyplaceOrchestration.composer.sendFailed')}: {sendError}
          </p>
        ) : null}
        <form className="mt-3 flex gap-2 border-t border-line pt-3" onSubmit={submit}>
          <input
            value={body}
            onChange={e => setBody(e.target.value)}
            placeholder={t('orchPage.connections.replyPlaceholder')}
            data-testid="orch-session-reply-input"
            className="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-2 text-sm text-content outline-none transition focus:border-primary-500 focus:ring-2 focus:ring-primary-500/20"
          />
          <Button
            type="submit"
            variant="primary"
            size="sm"
            disabled={!body.trim() || sending}
            data-testid="orch-session-reply-send">
            {t('tinyplaceOrchestration.composer.send')}
          </Button>
        </form>
      </SectionCard>
    </div>
  );
}

// ── One connection row + its nested sessions ─────────────────────────────────
function ConnectionRow({
  address,
  handle,
  sessions,
  expanded,
  onToggle,
  onOpenSession,
  onNewSession,
  creating,
}: {
  address: string;
  handle: string | null;
  sessions: SessionSummary[];
  expanded: boolean;
  onToggle: () => void;
  onOpenSession: (s: SessionSummary) => void;
  onNewSession: () => void;
  creating: boolean;
}) {
  const { t } = useT();
  const online = sessions.some(s => s.active);
  return (
    <li className="py-1" data-testid={`orch-connection-${address}`}>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={expanded}
        className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left transition hover:bg-surface-hover">
        <span className="flex-none text-[10px] text-content-muted">{expanded ? '▾' : '▸'}</span>
        <span className="relative flex-none">
          <span className="flex h-9 w-9 items-center justify-center rounded-full border border-line bg-surface-strong text-[11px] font-semibold text-content-secondary">
            {(handle ?? address).slice(0, 2)}
          </span>
          {online ? (
            <span className="absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-surface bg-sage-500" />
          ) : null}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-semibold text-content">
            {handle ? `@${handle}` : t('tinyplaceOrchestration.unknownSender')}
          </span>
          <span className="block truncate font-mono text-[11px] text-content-faint">
            {shortAddress(address)}
          </span>
        </span>
        <span className="flex-none rounded-full bg-surface-strong px-2 py-0.5 text-[10px] font-medium text-content-muted">
          {sessions.length === 0
            ? t('orchPage.connections.noSessions')
            : t('orchPage.connections.sessionCount').replace('{n}', String(sessions.length))}
        </span>
      </button>

      {expanded ? (
        <div className="ml-9 mt-1 space-y-1 pl-2">
          {sessions.map(session => {
            const meta = statusMeta(session.status, t);
            const label = session.label?.trim() || session.sessionId;
            return (
              <button
                key={session.sessionId}
                type="button"
                data-testid={`orch-session-${session.sessionId}`}
                onClick={() => onOpenSession(session)}
                className="flex w-full items-center gap-3 rounded-lg border border-line bg-surface-subtle px-3 py-2 text-left transition hover:bg-surface-hover">
                <span className={`h-1.5 w-1.5 flex-none rounded-full ${meta.dot}`} />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-2">
                    <span className="truncate text-sm font-medium text-content">{label}</span>
                    <span className={`flex-none text-[10px] font-medium ${meta.tone}`}>
                      {meta.label}
                    </span>
                  </span>
                  {session.currentTask ? (
                    <span className="mt-0.5 block truncate text-[11px] text-content-muted">
                      {session.currentTask}
                    </span>
                  ) : null}
                </span>
                <span className="flex-none text-[10px] text-content-faint">
                  {session.messageCount
                    ? t('orchPage.connections.messageCount').replace(
                        '{n}',
                        String(session.messageCount)
                      )
                    : ''}
                </span>
                <span className="flex-none text-content-faint">›</span>
              </button>
            );
          })}
          <button
            type="button"
            data-testid={`orch-new-session-${address}`}
            disabled={creating}
            onClick={onNewSession}
            className="flex w-full items-center gap-1 rounded-lg px-3 py-1.5 text-left text-[11px] font-medium text-primary-600 transition hover:bg-surface-hover disabled:opacity-50 dark:text-primary-300">
            + {t('tinyplaceOrchestration.newSession')}
          </button>
        </div>
      ) : null}
    </li>
  );
}

export default function ConnectionsPanel({
  onDiscover,
  onInitializeAgent,
}: {
  onDiscover?: () => void;
  /** Jump to the agent chat to describe + spin up a new sub-agent. */
  onInitializeAgent?: () => void;
}) {
  const { t } = useT();
  const { state, runAction, pendingAction, actionError } = usePairing();
  const sessions = useContactSessions();
  const [handles, setHandles] = useState<Record<string, string | null>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [open, setOpen] = useState<{ address: string; session: SessionSummary } | null>(null);
  const [creating, setCreating] = useState<string | null>(null);

  const snapshot = state.status === 'ok' ? state.snapshot : null;
  const accepted = useMemo(
    () => (snapshot?.contacts.contacts ?? []).filter(c => c.status === 'accepted'),
    [snapshot?.contacts.contacts]
  );
  const incoming = useMemo(() => snapshot?.requests.incoming ?? [], [snapshot?.requests.incoming]);
  const stats = snapshot?.stats ?? null;
  const totalSessions = sessions.sessions.length;

  // Best-effort @handle resolution (address always shown).
  const addressesKey = [...accepted, ...incoming].map(contactAddress).filter(Boolean).join(',');
  useEffect(() => {
    const ids = addressesKey ? Array.from(new Set(addressesKey.split(','))) : [];
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
      setHandles(prev => {
        const next = { ...prev };
        for (const [id, handle] of entries) if (!(id in next)) next[id] = handle;
        return next;
      });
    });
    return () => {
      cancelled = true;
    };
  }, [addressesKey]);

  const toggle = useCallback((address: string) => {
    setExpanded(prev => {
      const next = new Set(prev);
      if (next.has(address)) next.delete(address);
      else next.add(address);
      return next;
    });
  }, []);

  const newSession = useCallback(
    (address: string) => {
      setCreating(address);
      void runAction(`new:${address}`, async () => {
        const { session } = await orchestrationClient.sessionsCreate({ agentId: address });
        await sessions.refresh();
        setOpen({ address, session });
      }).finally(() => setCreating(null));
    },
    [runAction, sessions]
  );

  if (state.status === 'loading') {
    return (
      <p
        className="py-8 text-center text-sm text-content-muted"
        data-testid="orch-connections-loading">
        {t('tinyplaceOrchestration.loading')}
      </p>
    );
  }
  if (state.status === 'payment_required') {
    return (
      <p className="py-8 text-center text-sm text-amber-600 dark:text-amber-300">
        {t('tinyplaceOrchestration.paymentRequired')}
      </p>
    );
  }
  if (state.status === 'error') {
    return (
      <p className="py-8 text-center text-sm text-coral-600 dark:text-coral-300">
        {t('tinyplaceOrchestration.failedToLoad')}: {state.message}
      </p>
    );
  }

  if (open) {
    // Prefer the live session row (socket-refreshed) so the header's
    // status / message count / label stay current; fall back to the captured
    // snapshot for a just-created session not yet in the list.
    const liveSession =
      sessions.sessions.find(s => s.sessionId === open.session.sessionId) ?? open.session;
    return (
      <SessionView
        session={liveSession}
        contactAddr={open.address}
        handle={handles[open.address] ?? null}
        onBack={() => setOpen(null)}
      />
    );
  }

  return (
    <div className="space-y-4" data-testid="orch-connections-panel">
      <div className="grid grid-cols-3 gap-3">
        <StatTile
          label={t('orchPage.connections.statContacts')}
          value={stats?.contactCount ?? accepted.length}
          testId="orch-connections-stat-contacts"
        />
        <StatTile
          label={t('orchPage.connections.statSessions')}
          value={totalSessions}
          testId="orch-connections-stat-sessions"
        />
        <StatTile
          label={t('orchPage.connections.statPending')}
          value={(stats?.pendingIncoming ?? 0) + (stats?.pendingOutgoing ?? 0)}
          hint={t('orchPage.connections.pendingHint')}
          testId="orch-connections-stat-pending"
        />
      </div>

      {actionError && (
        <p className="rounded-md bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
          {actionError}
        </p>
      )}

      {incoming.length > 0 ? (
        <SectionCard
          title={t('tinyplaceOrchestration.pairing.requests')}
          testId="orch-connections-requests">
          <ul className="divide-y divide-line">
            {incoming.map(request => {
              const address = contactAddress(request);
              const handle = handles[address];
              return (
                <li key={address} className="flex items-center justify-between gap-3 py-2.5">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium text-content">
                      {handle ? `@${handle}` : t('tinyplaceOrchestration.unknownSender')}
                    </p>
                    <p className="truncate font-mono text-[11px] text-content-faint">
                      {shortAddress(address)}
                    </p>
                  </div>
                  <div className="flex flex-none gap-1.5">
                    <Button
                      variant="primary"
                      size="sm"
                      disabled={pendingAction !== null || !address}
                      onClick={() =>
                        void runAction(`accept:${address}`, () =>
                          apiClient.orchestrationPairing.acceptRequest(address)
                        )
                      }>
                      {t('tinyplaceOrchestration.pairing.accept')}
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={pendingAction !== null || !address}
                      onClick={() =>
                        void runAction(`decline:${address}`, () =>
                          apiClient.orchestrationPairing.declineRequest(address)
                        )
                      }>
                      {t('tinyplaceOrchestration.pairing.decline')}
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
        </SectionCard>
      ) : null}

      <SectionCard
        title={t('orchPage.connections.title')}
        description={t('orchPage.connections.description')}>
        {accepted.length === 0 ? (
          <div className="py-6 text-center" data-testid="orch-connections-empty">
            <p className="text-sm text-content-muted">{t('orchPage.connections.empty')}</p>
            {onDiscover && (
              <Button
                variant="primary"
                size="sm"
                className="mt-3"
                onClick={onDiscover}
                data-testid="orch-connections-empty-cta">
                {t('orchPage.connections.emptyCta')}
              </Button>
            )}
          </div>
        ) : (
          <ul className="divide-y divide-line/60">
            {accepted.map(contact => {
              const address = contactAddress(contact);
              return (
                <ConnectionRow
                  key={address}
                  address={address}
                  handle={handles[address] ?? null}
                  sessions={sessions.byContact.get(address) ?? []}
                  expanded={expanded.has(address)}
                  onToggle={() => toggle(address)}
                  onOpenSession={session => setOpen({ address, session })}
                  onNewSession={() => newSession(address)}
                  creating={creating === address}
                />
              );
            })}
          </ul>
        )}
      </SectionCard>

      {/* Initialize a brand-new sub-agent / instance. Unlike Discover (which
          links to an EXISTING peer), this explains how to spin up your own. */}
      <SectionCard
        title={t('orchPage.connections.initTitle')}
        description={t('orchPage.connections.initDesc')}
        testId="orch-connections-initialize">
        <Button
          variant="primary"
          size="sm"
          onClick={onInitializeAgent}
          disabled={!onInitializeAgent}
          data-testid="orch-connections-initialize-cta">
          {t('orchPage.connections.initCta')}
        </Button>
      </SectionCard>
    </div>
  );
}
