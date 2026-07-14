/**
 * ActiveSubagentsRail — the sidebar's "Active sub-agents" list, grouped by
 * instance.
 *
 * An **instance** is a contact (peer `agentId`) you coordinate with; its
 * **sub-agents** are the sessions running under it. Each instance shows an
 * avatar icon + a connection-status dot (aggregated over its sessions), and
 * expands to its sub-agents — each with its own status dot. Clicking a sub-agent
 * opens that session's full chat subpage.
 *
 * Status dot: green = connected, amber = waiting for input, red = errored,
 * hollow = disconnected. The instance dot surfaces the most actionable state
 * across its sessions (waiting › errored › connected › disconnected).
 */
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { SessionSummary } from '../../lib/orchestration/orchestrationClient';

type ConnState = 'connected' | 'waiting' | 'error' | 'disconnected';

/** Filled dot colour per state; `null` renders a hollow ring (disconnected). */
const DOT_CLASS: Record<ConnState, string | null> = {
  connected: 'bg-sage-500',
  waiting: 'bg-amber-500',
  error: 'bg-coral-500',
  disconnected: null,
};

const LABEL_KEY: Record<ConnState, string> = {
  connected: 'orchPage.sessions.statusConnected',
  waiting: 'orchPage.sessions.statusWaiting',
  error: 'orchPage.connections.status.error',
  disconnected: 'orchPage.sessions.statusDisconnected',
};

function sessionState(session: SessionSummary): ConnState {
  // Work-state semantics win (they carry their own colour).
  if (session.status === 'waiting-approval') return 'waiting';
  if (session.status === 'errored') return 'error';
  // Live peer presence when the core is confident; else the recency heuristic.
  if (session.peerOnline === true) return 'connected';
  if (session.peerOnline === false) return 'disconnected';
  if (session.active || session.status === 'running') return 'connected';
  return 'disconnected';
}

/** Most actionable state across an instance's sessions. */
function instanceState(sessions: SessionSummary[]): ConnState {
  const states = sessions.map(sessionState);
  if (states.includes('waiting')) return 'waiting';
  if (states.includes('error')) return 'error';
  if (states.includes('connected')) return 'connected';
  return 'disconnected';
}

function shortAddress(address: string): string {
  return address.length <= 14 ? address : `${address.slice(0, 6)}…${address.slice(-4)}`;
}

function sessionLabel(session: SessionSummary): string {
  return session.label?.trim() || session.sessionId;
}

function StatusDot({ state }: { state: ConnState }) {
  const { t } = useT();
  const fill = DOT_CLASS[state];
  return (
    <span
      title={t(LABEL_KEY[state])}
      aria-label={t(LABEL_KEY[state])}
      className={`h-2 w-2 flex-none rounded-full ${fill ?? 'border border-content-faint/60'}`}
    />
  );
}

export interface ActiveSubagentsRailProps {
  /** Sessions grouped by contact (`agentId`) — the instances. */
  byContact: Map<string, SessionSummary[]>;
  /** Currently open session id (highlighted when on the agent tab). */
  openSessionId: string | null;
  /** Whether the agent chat tab is active (so a selected row reads as current). */
  isAgentTab: boolean;
  onOpenSession: (sessionId: string) => void;
}

export default function ActiveSubagentsRail({
  byContact,
  openSessionId,
  isAgentTab,
  onOpenSession,
}: ActiveSubagentsRailProps) {
  const { t } = useT();
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const toggle = useCallback((address: string) => {
    setCollapsed(prev => {
      const next = new Set(prev);
      if (next.has(address)) next.delete(address);
      else next.add(address);
      return next;
    });
  }, []);

  const instances = Array.from(byContact.entries());

  return (
    <div className="mt-2 border-t border-line-subtle pt-2">
      <div className="px-2 pb-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-content-muted">
          {t('orchPage.sessions.railTitle')}
        </span>
      </div>

      {instances.length === 0 ? (
        <p className="px-2 py-1 text-[11px] text-content-faint">{t('orchPage.sessions.empty')}</p>
      ) : (
        <ul>
          {instances.map(([address, sessions]) => {
            const isOpen = !collapsed.has(address);
            return (
              <li key={address} data-testid={`orch-instance-${address}`}>
                {/* Instance header — icon + aggregate status dot + name. */}
                <button
                  type="button"
                  aria-expanded={isOpen}
                  data-testid={`orch-instance-toggle-${address}`}
                  onClick={() => toggle(address)}
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-content-secondary transition-colors hover:bg-surface-hover hover:text-content">
                  <span className="flex-none text-[9px] text-content-faint">
                    {isOpen ? '▾' : '▸'}
                  </span>
                  <span className="relative flex-none">
                    <span className="flex h-6 w-6 items-center justify-center rounded-md border border-line bg-surface-strong text-[10px] font-semibold text-content-secondary">
                      {address.slice(0, 2).toUpperCase()}
                    </span>
                    <span className="absolute -bottom-0.5 -right-0.5 rounded-full bg-surface-muted p-[1px]">
                      <StatusDot state={instanceState(sessions)} />
                    </span>
                  </span>
                  <span className="min-w-0 flex-1 truncate font-mono text-[12px]">
                    {shortAddress(address)}
                  </span>
                  <span className="flex-none text-[10px] text-content-faint">
                    {sessions.length}
                  </span>
                </button>

                {/* Sub-agents nested under the instance. */}
                {isOpen ? (
                  <ul className="ml-4 border-l border-line-subtle pl-1.5">
                    {sessions.map(session => {
                      const active = isAgentTab && openSessionId === session.sessionId;
                      return (
                        <li key={session.sessionId}>
                          <button
                            type="button"
                            data-testid={`orch-session-rail-${session.sessionId}`}
                            aria-current={active ? 'true' : undefined}
                            onClick={() => onOpenSession(session.sessionId)}
                            className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors ${
                              active
                                ? 'bg-surface-subtle text-content'
                                : 'text-content-secondary hover:bg-surface-hover hover:text-content'
                            }`}>
                            <StatusDot state={sessionState(session)} />
                            <span className="min-w-0 flex-1 truncate text-[13px]">
                              {sessionLabel(session)}
                            </span>
                            {session.unread > 0 ? (
                              <span className="flex-none rounded-full bg-primary-500 px-1.5 py-0.5 text-[10px] font-semibold text-content-inverted">
                                {session.unread}
                              </span>
                            ) : null}
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
