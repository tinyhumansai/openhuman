/**
 * DirectorySection — Agent World Directory section.
 *
 * Ported from tiny.place `website/src/components/explore/Directory.tsx`. Renders
 * a browsable grid of agents registered in the tiny.place directory inside the
 * standard `PanelScaffold` chrome (section title comes from the sidebar). Each
 * card shows the agent's handle, description, follower count, and skills/tags.
 * Authenticated users can follow/unfollow agents directly from the card.
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
    debug('fetching directory agents');

    void apiClient.directory
      .listAgents()
      .then(data => {
        if (cancelled) return;
        debug('[tinyplace][ui] DirectorySection: loaded %d agents', data.agents.length);
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

// Max followees pulled in the single batch lookup below. The directory rarely
// exceeds this; anything beyond it falls back to "not following" (the user can
// still follow, which corrects the state optimistically).
const FOLLOWING_FETCH_LIMIT = 500;

/**
 * Fetch the current agent's *following* list ONCE and expose it as a Set of
 * followee ids. This replaces the previous per-card `follows.followers()` call
 * (one request per directory card → N+1 / rate-limit pressure) with a single
 * request, letting each card derive its follow-state locally.
 *
 * Returns `null` until myAgentId is known and the fetch resolves, so cards can
 * distinguish "still loading" from "not following".
 */
function useMyFollowing(myAgentId: string | null): Set<string> | null {
  const [following, setFollowing] = useState<Set<string> | null>(null);
  useEffect(() => {
    if (!myAgentId) return;
    let cancelled = false;
    debug('[tinyplace][ui] DirectorySection: fetching following-set for %s', myAgentId);
    void apiClient.follows
      .following(myAgentId, { limit: FOLLOWING_FETCH_LIMIT })
      .then(res => {
        if (cancelled) return;
        const set = new Set(res.following.map(f => f.followee));
        debug('[tinyplace][ui] DirectorySection: following-set size=%d', set.size);
        setFollowing(set);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        // Treat a failed lookup as "follow nobody" so the UI still renders
        // Follow buttons instead of getting stuck in the loading state.
        debug('[tinyplace][ui] DirectorySection: following-set failed: %s', String(err));
        setFollowing(new Set());
      });
    return () => {
      cancelled = true;
    };
  }, [myAgentId]);
  return following;
}

// ── Sub-components ────────────────────────────────────────────────────────────

const CARD_CLASS =
  'rounded-lg border border-stone-200 bg-white dark:border-neutral-800 dark:bg-neutral-900';

function LoadingSkeleton() {
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className={`animate-pulse p-3 ${CARD_CLASS}`}>
          <div className="flex items-start gap-2.5">
            <div className="h-8 w-8 flex-shrink-0 rounded-full bg-stone-200 dark:bg-neutral-800" />
            <div className="min-w-0 flex-1 space-y-2">
              <div className="h-4 w-20 rounded bg-stone-200 dark:bg-neutral-800" />
              <div className="h-3 w-full rounded bg-stone-200 dark:bg-neutral-800" />
              <div className="flex gap-1">
                <div className="h-4 w-12 rounded-full bg-stone-200 dark:bg-neutral-800" />
                <div className="h-4 w-14 rounded-full bg-stone-200 dark:bg-neutral-800" />
              </div>
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function AgentCardItem({
  agent,
  myAgentId,
  followingSet,
}: {
  agent: AgentCard;
  myAgentId: string | null;
  // Set of followee ids the current agent follows; null while still loading.
  // Sourced once by the parent (see useMyFollowing) instead of per-card.
  followingSet: Set<string> | null;
}) {
  const [selected, setSelected] = useState(false);
  // Optimistic override applied after the user (un)follows from this card; null
  // means "defer to followingSet". Avoids a re-fetch just to reflect our click.
  const [localFollow, setLocalFollow] = useState<'following' | 'not_following' | null>(null);
  const [followerCount, setFollowerCount] = useState<number | null>(null);
  const [actionLoading, setActionLoading] = useState(false);
  const handle = getHandle(agent);
  const skills = getSkills(agent);
  const isSelf = myAgentId != null && agent.agentId === myAgentId;

  // Follow-state derived from the batched following-set (or our optimistic
  // override). 'unknown' = still loading, which hides the Follow button.
  const followState: 'unknown' | 'following' | 'not_following' =
    localFollow ??
    (followingSet == null
      ? 'unknown'
      : followingSet.has(agent.agentId)
        ? 'following'
        : 'not_following');

  // Fetch follow stats on mount.
  // NOTE: follower COUNT is still one request per card — `directory.listAgents`
  // doesn't return counts. Eliminating these needs a backend/GraphQL directory
  // query that embeds followerCount (tracked separately); the per-card follower
  // *list* lookup has already been removed in favour of the batched set above.
  useEffect(() => {
    let cancelled = false;
    void apiClient.follows
      .stats(agent.agentId)
      .then(stats => {
        if (cancelled) return;
        setFollowerCount(stats.followerCount);
      })
      .catch(() => {
        // Stats unavailable -- leave null (hidden).
      });
    return () => {
      cancelled = true;
    };
  }, [agent.agentId]);

  const handleFollow = useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation();
      if (actionLoading || !myAgentId) return;
      setActionLoading(true);
      try {
        if (followState === 'following') {
          await apiClient.follows.unfollow(agent.agentId);
          setLocalFollow('not_following');
          setFollowerCount(c => (c != null ? Math.max(0, c - 1) : c));
          debug('unfollowed %s', agent.agentId);
        } else {
          await apiClient.follows.follow(agent.agentId);
          setLocalFollow('following');
          setFollowerCount(c => (c != null ? c + 1 : c));
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
          : 'hover:border-stone-300 dark:hover:border-neutral-700',
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
            <p className="truncate text-sm font-medium text-stone-900 dark:text-neutral-100">
              {handle}
            </p>
            {!isSelf && myAgentId && followState !== 'unknown' && (
              <button
                type="button"
                className={[
                  'ml-2 flex-shrink-0 rounded-full px-2.5 py-0.5 text-xs font-medium transition-colors',
                  followState === 'following'
                    ? 'bg-stone-200 text-stone-700 hover:bg-red-100 hover:text-red-700 dark:bg-neutral-700 dark:text-neutral-200 dark:hover:bg-red-900/30 dark:hover:text-red-400'
                    : 'bg-primary-600 text-white hover:bg-primary-700',
                ].join(' ')}
                disabled={actionLoading}
                onClick={handleFollow}>
                {actionLoading ? '...' : followState === 'following' ? 'Following' : 'Follow'}
              </button>
            )}
          </div>
          <p className="mt-0.5 truncate text-xs text-stone-500 dark:text-neutral-400">
            {agent.description ?? ''}
          </p>
          {followerCount != null && (
            <p className="mt-0.5 text-xs text-stone-400 dark:text-neutral-500">
              {followerCount} {followerCount === 1 ? 'follower' : 'followers'}
            </p>
          )}
          {skills.length > 0 && (
            <div className="mt-1.5 flex flex-wrap gap-1">
              {skills.map(skill => (
                <span
                  key={skill}
                  className="rounded-full bg-stone-100 px-1.5 py-0.5 text-xs text-stone-600 dark:bg-neutral-800 dark:text-neutral-300">
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
      {body && <p className="max-w-md text-sm text-stone-500 dark:text-neutral-400">{body}</p>}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export default function DirectorySection() {
  const state = useDirectoryAgents();
  const myAgentId = useMyAgentId();
  // One batched lookup of who we follow, shared by every card below.
  const followingSet = useMyFollowing(myAgentId);

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
        tone="text-stone-700 dark:text-neutral-200"
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
          tone="text-stone-600 dark:text-neutral-300"
          title="No agents found"
          body="No agents are registered in the directory yet."
        />
      ) : (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {agents.map(agent => (
            <AgentCardItem
              key={agent.agentId}
              agent={agent}
              myAgentId={myAgentId}
              followingSet={followingSet}
            />
          ))}
        </div>
      );
  }

  return (
    <PanelScaffold description="Browse agents in the tiny.place directory">{body}</PanelScaffold>
  );
}
