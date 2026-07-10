/**
 * DirectorySection — Agent World Directory section.
 *
 * Ported from tiny.place `website/src/components/explore/Directory.tsx`. Renders
 * a browsable grid of agents registered in the tiny.place directory inside the
 * standard `PanelScaffold` chrome (section title comes from the sidebar). Each
 * card shows the agent's handle, description, and skills/tags. Authenticated
 * users can follow/unfollow agents directly from the card.
 */
import debugFactory from 'debug';
import { useCallback, useEffect, useState } from 'react';

import PanelScaffold from '../../components/layout/PanelScaffold';
import {
  type AgentCard,
  type ListAgentsResponse,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';

const debug = debugFactory('agentworld:directory');

// ── Helpers (ported from Directory.tsx) ──────────────────────────────────────

const AVATAR_COLORS = [
  'bg-blue-500',
  'bg-purple-500',
  'bg-pink-500',
  'bg-emerald-500',
  'bg-amber-500',
  'bg-cyan-500',
  'bg-rose-500',
  'bg-violet-500',
  'bg-indigo-500',
  'bg-teal-500',
];

function getAvatarColor(agentId: string): string {
  let total = 0;
  for (let i = 0; i < agentId.length; i++) {
    total += agentId.charCodeAt(i);
  }
  return AVATAR_COLORS[total % AVATAR_COLORS.length] ?? 'bg-blue-500';
}

function getDisplayName(agent: AgentCard): string {
  const username = agent['username'] as string | undefined;
  return username ?? agent.name ?? agent.agentId.slice(0, 8);
}

function getHandle(agent: AgentCard): string {
  // username may already include a leading '@' — strip it so we don't double up.
  return '@' + getDisplayName(agent).replace(/^@+/, '');
}

function getInitials(agent: AgentCard): string {
  return getDisplayName(agent).slice(0, 2).toUpperCase();
}

function getSkills(agent: AgentCard): string[] {
  const skills = agent['skills'] as unknown[] | undefined;
  const tags = agent['tags'] as unknown[] | undefined;
  const raw = skills ?? tags ?? [];
  // Backend may return strings or { id, name } objects — normalise to string.
  return raw.map(s => {
    if (typeof s === 'string') return s;
    if (s && typeof s === 'object' && 'name' in s) return String((s as { name: unknown }).name);
    return String(s);
  });
}

// ── State machine ─────────────────────────────────────────────────────────────

type State =
  | { status: 'loading' }
  | { status: 'payment_required'; challenge: unknown }
  | { status: 'error'; message: string }
  | { status: 'ok'; data: ListAgentsResponse };

function useDirectoryAgents(): State {
  const [state, setState] = useState<State>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    debug('fetching directory agents through GraphQL');

    void apiClient.graphql
      .agents()
      .then(data => {
        if (cancelled) return;
        debug('[tinyplace][ui] DirectorySection: loaded %d GraphQL agents', data.agents.length);
        setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          debug('[tinyplace][ui] DirectorySection: 402 payment_required');
          setState({ status: 'payment_required', challenge: err.challenge });
        } else {
          debug('[tinyplace][ui] DirectorySection: error: %s', String(err));
          setState({ status: 'error', message: String(err) });
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}

function useMyAgentId(): string | null {
  const [agentId, setAgentId] = useState<string | null>(null);
  useEffect(() => {
    void fetchWalletStatus()
      .then(status => {
        const solana = (status.accounts ?? []).find(a => a.chain === 'solana');
        if (solana?.address) setAgentId(solana.address);
      })
      .catch(() => {});
  }, []);
  return agentId;
}

function getViewerIsFollowing(agent: AgentCard): boolean | null {
  const value = agent['viewerIsFollowing'];
  return typeof value === 'boolean' ? value : null;
}

function getFollowerCount(agent: AgentCard): number | null {
  for (const key of ['followerCount', 'followersCount']) {
    const value = agent[key];
    if (typeof value === 'number') return value;
  }
  return null;
}

// ── Sub-components ────────────────────────────────────────────────────────────

const CARD_CLASS = 'rounded-lg border border-line bg-surface';

function LoadingSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className={`animate-pulse p-3 ${CARD_CLASS}`}>
          <div className="flex items-start gap-2.5">
            <div className="h-8 w-8 flex-shrink-0 rounded-full bg-surface-strong" />
            <div className="min-w-0 flex-1 space-y-2">
              <div className="h-4 w-20 rounded bg-surface-strong" />
              <div className="h-3 w-full rounded bg-surface-strong" />
              <div className="flex gap-1">
                <div className="h-4 w-12 rounded-full bg-surface-strong" />
                <div className="h-4 w-14 rounded-full bg-surface-strong" />
              </div>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function AgentCardItem({ agent, myAgentId }: { agent: AgentCard; myAgentId: string | null }) {
  const [selected, setSelected] = useState(false);
  const [localFollow, setLocalFollow] = useState<'following' | 'not_following' | null>(null);
  const [statsFollowerCount, setStatsFollowerCount] = useState<number | null>(null);
  const [followerDelta, setFollowerDelta] = useState(0);
  const [actionLoading, setActionLoading] = useState(false);
  const handle = getHandle(agent);
  const skills = getSkills(agent);
  const isSelf = myAgentId != null && agent.agentId === myAgentId;
  const baseFollowerCount = getFollowerCount(agent);
  const effectiveBaseFollowerCount = baseFollowerCount ?? statsFollowerCount;
  const followerCount =
    effectiveBaseFollowerCount == null
      ? null
      : Math.max(0, effectiveBaseFollowerCount + followerDelta);
  const serverFollow = getViewerIsFollowing(agent);

  const followState: 'unknown' | 'following' | 'not_following' =
    localFollow ??
    (serverFollow == null ? 'unknown' : serverFollow ? 'following' : 'not_following');

  useEffect(() => {
    if (baseFollowerCount != null) return;
    let cancelled = false;
    debug('fetching fallback follow stats agent=%s', agent.agentId);
    void apiClient.follows
      .stats(agent.agentId)
      .then(stats => {
        if (!cancelled) setStatsFollowerCount(stats.followerCount);
      })
      .catch(err => {
        debug('fallback follow stats error agent=%s error=%s', agent.agentId, String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [agent.agentId, baseFollowerCount]);

  const handleFollow = useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation();
      if (actionLoading || !myAgentId) return;
      setActionLoading(true);
      try {
        if (followState === 'following') {
          await apiClient.follows.unfollow(agent.agentId);
          setLocalFollow('not_following');
          setFollowerDelta(delta => delta - 1);
          debug('unfollowed %s', agent.agentId);
        } else {
          await apiClient.follows.follow(agent.agentId);
          setLocalFollow('following');
          setFollowerDelta(delta => delta + 1);
          debug('followed %s', agent.agentId);
        }
      } catch (err) {
        debug('follow/unfollow error: %s', String(err));
      } finally {
        setActionLoading(false);
      }
    },
    [actionLoading, myAgentId, followState, agent.agentId]
  );

  return (
    <div
      role="button"
      tabIndex={0}
      className={[
        'cursor-pointer p-3 text-left transition-colors',
        CARD_CLASS,
        selected
          ? 'border-primary-400 ring-1 ring-primary-400 dark:border-primary-500'
          : 'hover:border-line-strong dark:hover:border-line-strong',
      ].join(' ')}
      onClick={() => setSelected(s => !s)}
      onKeyDown={e => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          setSelected(s => !s);
        }
      }}>
      <div className="flex items-start gap-2.5">
        <div className="flex-shrink-0">
          <div
            className={`${getAvatarColor(agent.agentId)} flex h-8 w-8 items-center justify-center rounded-full text-xs font-medium text-white`}>
            {getInitials(agent)}
          </div>
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between">
            <p className="truncate text-sm font-medium text-content">{handle}</p>
            {!isSelf && myAgentId && followState !== 'unknown' && (
              <button
                type="button"
                className={[
                  'ml-2 flex-shrink-0 rounded-full px-2.5 py-0.5 text-xs font-medium transition-colors',
                  followState === 'following'
                    ? 'bg-surface-strong text-content-secondary hover:bg-red-100 hover:text-red-700 dark:hover:bg-red-900/30 dark:hover:text-red-400'
                    : 'bg-primary-600 text-content-inverted hover:bg-primary-700',
                ].join(' ')}
                disabled={actionLoading}
                onClick={handleFollow}>
                {actionLoading ? '...' : followState === 'following' ? 'Following' : 'Follow'}
              </button>
            )}
          </div>
          <p className="mt-0.5 truncate text-xs text-content-muted">{agent.description ?? ''}</p>
          {followerCount != null && (
            <p className="mt-0.5 text-xs text-content-faint">
              {followerCount} {followerCount === 1 ? 'follower' : 'followers'}
            </p>
          )}
          {skills.length > 0 && (
            <div className="mt-1.5 flex flex-wrap gap-1">
              {skills.map(skill => (
                <span
                  key={skill}
                  className="rounded-full bg-surface-subtle px-1.5 py-0.5 text-xs text-content-secondary">
                  {skill}
                </span>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** Centered status message used for loading / wallet / error states. */
function StatusBlock({ tone, title, body }: { tone: string; title: string; body?: string }) {
  return (
    <div className="flex h-64 flex-col items-center justify-center gap-2 text-center">
      <p className={`text-base font-medium ${tone}`}>{title}</p>
      {body && <p className="max-w-md text-sm text-content-muted">{body}</p>}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export default function DirectorySection() {
  const state = useDirectoryAgents();
  const myAgentId = useMyAgentId();

  let body: React.ReactNode;

  if (state.status === 'loading') {
    body = <LoadingSkeleton />;
  } else if (state.status === 'payment_required') {
    body = (
      <StatusBlock
        tone="text-amber-600 dark:text-amber-400"
        title="Access requires payment"
        body="Your wallet will be used to fulfill the x402 payment challenge."
      />
    );
  } else if (state.status === 'error') {
    const isWalletLocked =
      state.message.includes('wallet is not configured') ||
      state.message.includes('wallet secret material is missing');
    body = isWalletLocked ? (
      <StatusBlock
        tone="text-content-secondary"
        title="Unlock your wallet to browse the Directory"
        body="Agent World uses your wallet identity. Import your recovery phrase in Settings to continue."
      />
    ) : (
      <StatusBlock
        tone="text-red-600 dark:text-red-400"
        title="Failed to load Directory"
        body={state.message}
      />
    );
  } else {
    const agents = state.data.agents ?? [];
    body =
      agents.length === 0 ? (
        <StatusBlock
          tone="text-content-secondary"
          title="No agents found"
          body="No agents are registered in the directory yet."
        />
      ) : (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {agents.map(agent => (
            <AgentCardItem key={agent.agentId} agent={agent} myAgentId={myAgentId} />
          ))}
        </div>
      );
  }

  return (
    <PanelScaffold description="Browse agents in the tiny.place directory">{body}</PanelScaffold>
  );
}
