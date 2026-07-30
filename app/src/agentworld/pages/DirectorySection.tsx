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
import { useCallback, useEffect, useRef, useState } from 'react';

import PanelScaffold from '../../components/layout/PanelScaffold';
import { useDebouncedValue } from '../../hooks/useDebouncedValue';
import { type AgentCard, PaymentRequiredError } from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import { apiClient } from '../AgentWorldShell';
import AgentProfileModal from '../components/AgentProfileModal';
import StatusBlock from '../components/StatusBlock';
import { useMyAgentId } from '../hooks/useMyAgentId';
import { getAvatarColor, getHandle, getInitials, getSkills } from './directoryHelpers';

const debug = debugFactory('agentworld:directory');

// One page of directory results. Grid-friendly (divides 1/2/3-column layouts).
// Exported so the pagination tests can build a full page without magic numbers.
export const DIRECTORY_PAGE_SIZE = 24;

// ── State machine ─────────────────────────────────────────────────────────────

type State =
  | { status: 'loading' }
  | { status: 'payment_required'; challenge: unknown }
  | { status: 'error'; message: string }
  | {
      status: 'ok';
      agents: AgentCard[];
      /** Offset to request for the next "Load more" page. */
      nextOffset: number;
      hasMore: boolean;
      loadingMore: boolean;
      /** Non-null after a failed "Load more"; existing rows are preserved. */
      loadMoreError: string | null;
    };

/**
 * Fetches the directory grid with server-side search + offset pagination.
 *
 * `query` is the debounced search term (empty = browse all). It maps to the
 * GraphQL `query` variable via `graphql.agents({ q })`. A query change refetches
 * page 0 and replaces the list; `loadMore(offset)` appends the next page,
 * deduping by `agentId` so a mutation-shifted offset can't double-render a card.
 * End-of-list is inferred from a short page (`< DIRECTORY_PAGE_SIZE`), mirroring
 * the Feed/Ledger "Load more" pattern.
 */
function useDirectoryAgents(query: string): { state: State; loadMore: (offset: number) => void } {
  const [state, setState] = useState<State>({ status: 'loading' });
  // Guards late async resolutions from a "Load more" after unmount.
  const mountedRef = useRef(true);
  // Bumped on every page-0 fetch (i.e. every query change). A `loadMore`
  // captures the generation live at call time and drops its response if a newer
  // page-0 has since taken over — otherwise a stale in-flight "Load more" for
  // the previous query could append its agents onto a different search's result
  // set (the mounted check alone doesn't catch this; #5271 review).
  const genRef = useRef(0);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Page 0 — on mount and whenever the debounced query changes.
  useEffect(() => {
    let cancelled = false;
    genRef.current += 1;
    const q = query.trim() || undefined;
    // Log only whether a query is present, never the raw term — search text is
    // user-authored and may contain PII (AGENTS.md: never log full PII).
    setState({ status: 'loading' });
    debug('[agentworld:directory] fetching page 0 hasQuery=%s', q !== undefined);

    void apiClient.graphql
      .agents({ q, limit: DIRECTORY_PAGE_SIZE, offset: 0 })
      .then(result => {
        if (cancelled) return;
        // Reading `.length` throws if the payload omits `agents` → error branch
        // (preserves the existing malformed-payload contract).
        const page = result.agents;
        const hasMore = page.length >= DIRECTORY_PAGE_SIZE;
        debug('[agentworld:directory] loaded page 0 received=%d hasMore=%s', page.length, hasMore);
        setState({
          status: 'ok',
          agents: page,
          nextOffset: DIRECTORY_PAGE_SIZE,
          hasMore,
          loadingMore: false,
          loadMoreError: null,
        });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          debug('[agentworld:directory] page 0 payment_required');
          setState({ status: 'payment_required', challenge: err.challenge });
        } else {
          debug('[agentworld:directory] page 0 error: %s', String(err));
          setState({ status: 'error', message: String(err) });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [query]);

  // Append the next page. `offset` comes from the rendered 'ok' state so the
  // cursor stays a pure function of pages requested; reentry is prevented by
  // disabling the button while `loadingMore` is set.
  const loadMore = useCallback(
    (offset: number) => {
      const q = query.trim() || undefined;
      // Snapshot the generation so a query change (new page 0) invalidates this
      // in-flight request; see `genRef`.
      const gen = genRef.current;
      debug('[agentworld:directory] loading more offset=%d hasQuery=%s', offset, q !== undefined);
      setState(prev =>
        prev.status === 'ok' ? { ...prev, loadingMore: true, loadMoreError: null } : prev
      );

      void apiClient.graphql
        .agents({ q, limit: DIRECTORY_PAGE_SIZE, offset })
        .then(result => {
          if (!mountedRef.current || gen !== genRef.current) return;
          const page = result.agents ?? [];
          const hasMore = page.length >= DIRECTORY_PAGE_SIZE;
          setState(prev => {
            if (prev.status !== 'ok') return prev;
            // Dedupe by agentId: if the directory shifted between page fetches
            // the overlap must not produce duplicate React keys or double rows.
            const seen = new Set(prev.agents.map(a => a.agentId));
            const fresh = page.filter(a => !seen.has(a.agentId));
            debug(
              '[agentworld:directory] appended received=%d fresh=%d hasMore=%s',
              page.length,
              fresh.length,
              hasMore
            );
            return {
              status: 'ok',
              agents: [...prev.agents, ...fresh],
              nextOffset: offset + DIRECTORY_PAGE_SIZE,
              hasMore,
              loadingMore: false,
              loadMoreError: null,
            };
          });
        })
        .catch((err: unknown) => {
          if (!mountedRef.current || gen !== genRef.current) return;
          debug('[agentworld:directory] load more failed: %s', String(err));
          setState(prev =>
            prev.status === 'ok'
              ? { ...prev, loadingMore: false, loadMoreError: String(err) }
              : prev
          );
        });
    },
    [query]
  );

  return { state, loadMore };
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

function AgentCardItem({
  agent,
  myAgentId,
  onOpen,
}: {
  agent: AgentCard;
  myAgentId: string | null;
  /** Open this agent's profile (card click / Enter / Space). */
  onOpen: () => void;
}) {
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
        'hover:border-line-strong dark:hover:border-line-strong',
      ].join(' ')}
      onClick={onOpen}
      onKeyDown={e => {
        // Only handle keys targeting the card itself. Without this guard an
        // Enter/Space keydown on an inner control (e.g. the Follow button)
        // bubbles up here, gets preventDefault()'d — suppressing the button's
        // native activation — and opens the profile modal instead of
        // following/unfollowing (keyboard-a11y bug, #4927 review).
        if (e.target !== e.currentTarget) return;
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onOpen();
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

// ── Main component ────────────────────────────────────────────────────────────

export default function DirectorySection() {
  const { t } = useT();
  // Immediate input value; the fetch keys off the debounced copy so typing
  // doesn't fire a request per keystroke.
  const [query, setQuery] = useState('');
  const debouncedQuery = useDebouncedValue(query, 300);
  const { state, loadMore } = useDirectoryAgents(debouncedQuery);
  const myAgent = useMyAgentId();
  const myAgentId = myAgent.status === 'ready' ? myAgent.agentId : null;
  // The directory entry whose profile is open in the modal, or null when closed.
  const [openAgent, setOpenAgent] = useState<AgentCard | null>(null);
  const hasQuery = debouncedQuery.trim().length > 0;

  let body: React.ReactNode;

  if (state.status === 'loading') {
    body = <LoadingSkeleton />;
  } else if (state.status === 'payment_required') {
    body = (
      <StatusBlock
        tone="warning"
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
        tone="neutral"
        title="Unlock your wallet to browse the Directory"
        body="Agent World uses your wallet identity. Import your recovery phrase in Settings to continue."
      />
    ) : (
      <StatusBlock tone="danger" title="Failed to load Directory" body={state.message} />
    );
  } else if (state.agents.length === 0) {
    // A search that matched nothing reads differently from an empty directory.
    body = hasQuery ? (
      <StatusBlock
        tone="neutral"
        title={t('agentWorld.directory.noResults', 'No agents match your search.')}
        body={t('agentWorld.directory.noResultsHint', 'Try a different handle or name.')}
      />
    ) : (
      <StatusBlock
        tone="neutral"
        title={t('agentWorld.directory.empty', 'No agents found')}
        body={t('agentWorld.directory.emptyHint', 'No agents are registered in the directory yet.')}
      />
    );
  } else {
    body = (
      <>
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {state.agents.map(agent => (
            <AgentCardItem
              key={agent.agentId}
              agent={agent}
              myAgentId={myAgentId}
              onOpen={() => {
                debug('[agentworld:directory] opening profile for a directory entry');
                setOpenAgent(agent);
              }}
            />
          ))}
        </div>
        {state.hasMore && (
          <div className="mt-4 flex justify-center">
            <button
              type="button"
              disabled={state.loadingMore}
              onClick={() => loadMore(state.nextOffset)}
              className="rounded-md border border-line bg-surface px-4 py-1.5 text-sm font-medium text-content transition-colors hover:border-line-strong disabled:opacity-60">
              {state.loadingMore
                ? t('agentWorld.directory.loadingMore', 'Loading…')
                : t('agentWorld.directory.loadMore', 'Load more')}
            </button>
          </div>
        )}
        {state.loadMoreError && (
          <p className="mt-2 text-center text-xs text-red-600 dark:text-red-400">
            {t('agentWorld.directory.loadMoreError', "Couldn't load more agents.")}
          </p>
        )}
      </>
    );
  }

  return (
    <PanelScaffold description="Browse agents in the tiny.place directory">
      <div className="mb-3">
        <input
          type="search"
          value={query}
          onChange={e => setQuery(e.target.value)}
          aria-label={t('agentWorld.directory.searchLabel', 'Search agents')}
          placeholder={t(
            'agentWorld.directory.searchPlaceholder',
            'Search agents by handle or name'
          )}
          className="w-full rounded-md border border-line bg-surface px-3 py-1.5 text-sm text-content placeholder:text-content-faint focus:border-primary-500 focus:outline-none"
        />
      </div>
      {body}
      {openAgent && <AgentProfileModal agent={openAgent} onClose={() => setOpenAgent(null)} />}
    </PanelScaffold>
  );
}
