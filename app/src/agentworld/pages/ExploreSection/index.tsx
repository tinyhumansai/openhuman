/**
 * ExploreSection — Agent World "Explore" overview.
 *
 * Renders network stat cards at the top (via explorer.overview()) followed by
 * four live-data sections that each fetch independently:
 *   - Trending Communities  → apiClient.groups.list({ limit: 12 })
 *   - Active Jobs           → apiClient.graphql.jobs({ status: 'OPEN', limit: 6 })
 *   - Featured Bounties     → apiClient.graphql.bounties({ status: 'open', limit: 6 })
 *   - New Agents            → apiClient.graphql.agents({ limit: 8 })
 *
 * Each live section handles loading / empty / error independently; a failure in
 * one section never crashes the page. The stats section uses a StatusBlock for
 * wallet-locked / payment-required / hard error states.
 */
import debugFactory from 'debug';
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import PanelScaffold from '../../../components/layout/PanelScaffold';
import {
  type AgentCard,
  type ExplorerOverview,
  type GqlBounty,
  type GqlJobPosting,
  type GroupMetadata,
  PaymentRequiredError,
} from '../../../lib/agentworld/invokeApiClient';
import { useT } from '../../../lib/i18n/I18nContext';
import { apiClient } from '../../AgentWorldShell';
import { decimalsForAsset, resolveAssetSymbol } from '../../assets';
import { formatUnits } from '../../components/X402ConfirmDialog';

const debug = debugFactory('agentworld:explore');

// ── Shared card style ─────────────────────────────────────────────────────────

const CARD_CLASS = 'rounded-lg border border-line bg-surface';

function formatAmount(amount: string): string {
  if (!Number.isFinite(Number(amount))) return amount;
  const negative = amount.startsWith('-');
  const body = negative ? amount.slice(1) : amount;
  const [intPart, fracPart] = body.split('.');
  const grouped = Number(intPart).toLocaleString('en-US');
  const out = fracPart != null ? `${grouped}.${fracPart}` : grouped;
  return negative ? `-${out}` : out;
}

function formatReward(amount: string, asset: string): string {
  const decimals = decimalsForAsset(asset);
  const display = decimals > 0 ? formatUnits(amount, decimals) : amount;
  return `${formatAmount(display)} ${resolveAssetSymbol(asset)}`;
}

// ── Stats section types & hook ────────────────────────────────────────────────

type StatsState =
  | { status: 'loading' }
  | { status: 'payment_required'; challenge: unknown }
  | { status: 'error'; message: string }
  | { status: 'ok'; data: ExplorerOverview };

// OverviewShape removed — ExplorerOverview now carries typed allTime/last24h/ledger fields.

function useExplorerOverview(): StatsState {
  const [state, setState] = useState<StatsState>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    debug('fetching explorer overview');

    void apiClient.explorer
      .overview()
      .then(data => {
        if (cancelled) return;
        debug('loaded explorer overview');
        setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          debug('explorer overview: 402 payment_required');
          setState({ status: 'payment_required', challenge: err.challenge });
        } else {
          debug('explorer overview: error: %s', String(err));
          setState({ status: 'error', message: String(err) });
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}

// ── Live section state type ───────────────────────────────────────────────────

type SectionState<T> =
  | { status: 'loading' }
  | { status: 'ok'; data: T[] }
  | { status: 'empty' }
  | { status: 'error' };

// ── Communities hook ──────────────────────────────────────────────────────────

function useExploreCommunities(): SectionState<GroupMetadata> {
  const [state, setState] = useState<SectionState<GroupMetadata>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    debug('fetching explore communities');

    void apiClient.groups
      .list({ limit: 12 })
      .then(raw => {
        if (cancelled) return;
        // Sort client-side by memberCount desc (no server-side sort param).
        const sorted = [...raw].sort((a, b) => (b.memberCount ?? 0) - (a.memberCount ?? 0));
        if (sorted.length === 0) {
          debug('communities section: empty, hiding');
          setState({ status: 'empty' });
        } else {
          debug('loaded %d communities', sorted.length);
          setState({ status: 'ok', data: sorted });
        }
      })
      .catch(err => {
        if (cancelled) return;
        debug('communities fetch failed: %s', String(err));
        setState({ status: 'error' });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}

// ── Jobs hook ─────────────────────────────────────────────────────────────────

function useExploreJobs(): SectionState<GqlJobPosting> {
  const [state, setState] = useState<SectionState<GqlJobPosting>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    debug('fetching explore jobs');

    void apiClient.graphql
      .jobs({ status: 'OPEN', limit: 6 })
      .then(result => {
        if (cancelled) return;
        const jobs = result.jobs ?? [];
        if (jobs.length === 0) {
          debug('jobs section: empty, hiding');
          setState({ status: 'empty' });
        } else {
          debug('loaded %d jobs', jobs.length);
          setState({ status: 'ok', data: jobs });
        }
      })
      .catch(err => {
        if (cancelled) return;
        debug('jobs fetch failed: %s', String(err));
        setState({ status: 'error' });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}

// ── Bounties hook ─────────────────────────────────────────────────────────────

function useExploreBounties(): SectionState<GqlBounty> {
  const [state, setState] = useState<SectionState<GqlBounty>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    debug('fetching explore bounties');

    void apiClient.graphql
      .bounties({ status: 'open', limit: 6 })
      .then(result => {
        if (cancelled) return;
        // Client-side filter to open status in case the server ignores the param.
        const open = (result ?? []).filter(b => b.status === 'open');
        if (open.length === 0) {
          debug('bounties section: empty, hiding');
          setState({ status: 'empty' });
        } else {
          debug('loaded %d bounties', open.length);
          setState({ status: 'ok', data: open });
        }
      })
      .catch(err => {
        if (cancelled) return;
        debug('bounties fetch failed: %s', String(err));
        setState({ status: 'error' });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}

// ── Agents hook ───────────────────────────────────────────────────────────────

function useExploreAgents(): SectionState<AgentCard> {
  const [state, setState] = useState<SectionState<AgentCard>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    debug('fetching explore agents');

    void apiClient.graphql
      .agents({ limit: 8 })
      .then(result => {
        if (cancelled) return;
        const agents = result.agents ?? [];
        if (agents.length === 0) {
          debug('agents section: empty, hiding');
          setState({ status: 'empty' });
        } else {
          debug('loaded %d agents', agents.length);
          setState({ status: 'ok', data: agents });
        }
      })
      .catch(err => {
        if (cancelled) return;
        debug('agents fetch failed: %s', String(err));
        setState({ status: 'error' });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}

// ── Stat helper functions ─────────────────────────────────────────────────────

function usd(value?: string): string {
  if (value == null) return '—';
  const n = Number(value);
  if (Number.isNaN(n)) return `$${value}`;
  return `$${n.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

function num(value?: number): string {
  return value == null ? '—' : value.toLocaleString();
}

// ── Shared primitive components ───────────────────────────────────────────────

function StatCard({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className={`p-4 ${CARD_CLASS}`}>
      <div className="text-[11px] font-semibold uppercase tracking-wider text-content-muted">
        {label}
      </div>
      <div className="mt-1.5 text-2xl font-semibold text-content">{value}</div>
      {sub && <div className="mt-0.5 text-xs text-content-faint">{sub}</div>}
    </div>
  );
}

/** Centered status message for loading / wallet / hard error states of the stats block. */
function StatusBlock({ tone, title, body }: { tone: string; title: string; body?: string }) {
  return (
    <div className="flex h-64 flex-col items-center justify-center gap-2 text-center">
      <p className={`text-base font-medium ${tone}`}>{title}</p>
      {body && <p className="max-w-md text-sm text-content-muted">{body}</p>}
    </div>
  );
}

/** Section heading row with optional "View all" link. */
function SectionHeader({
  title,
  viewAllLabel,
  onViewAll,
}: {
  title: string;
  viewAllLabel: string;
  onViewAll: () => void;
}) {
  return (
    <div className="mb-3 flex items-center justify-between">
      <h3 className="text-sm font-semibold text-content">{title}</h3>
      <button
        type="button"
        onClick={onViewAll}
        className="text-xs text-ocean-500 hover:underline dark:text-blue-400">
        {viewAllLabel}
      </button>
    </div>
  );
}

/** Inline empty message rendered inside a section when it has no data. */
function SectionEmpty({ message }: { message: string }) {
  return <p className="py-4 text-center text-sm text-content-faint">{message}</p>;
}

// ── Communities section ───────────────────────────────────────────────────────

function CommunitySkeletonGrid() {
  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className={`animate-pulse p-3 ${CARD_CLASS}`}>
          <div className="space-y-2">
            <div className="h-4 w-3/4 rounded bg-surface-strong" />
            <div className="h-3 w-full rounded bg-surface-strong" />
            <div className="h-3 w-1/3 rounded bg-surface-strong" />
          </div>
        </div>
      ))}
    </div>
  );
}

function CommunityCard({ group }: { group: GroupMetadata }) {
  const tags = group.tags ?? [];
  return (
    <div className={`p-3 ${CARD_CLASS}`}>
      <div className="font-medium text-content">{group.name}</div>
      {group.description && (
        <p className="mt-1 line-clamp-2 text-xs text-content-muted">{group.description}</p>
      )}
      <div className="mt-2 flex flex-wrap items-center gap-1">
        <span className="text-xs text-content-faint">{group.memberCount} members</span>
        {tags.slice(0, 3).map(tag => (
          <span
            key={tag}
            className="rounded-full bg-surface-subtle px-2 py-0.5 text-[11px] text-content-secondary">
            {tag}
          </span>
        ))}
      </div>
    </div>
  );
}

function ExploreCommunitiesGrid({
  state,
  title,
  viewAllLabel,
  emptyMessage,
  onViewAll,
}: {
  state: SectionState<GroupMetadata>;
  title: string;
  viewAllLabel: string;
  emptyMessage: string;
  onViewAll: () => void;
}) {
  if (state.status === 'error') return null;

  return (
    <section>
      <SectionHeader title={title} viewAllLabel={viewAllLabel} onViewAll={onViewAll} />
      {state.status === 'loading' && <CommunitySkeletonGrid />}
      {state.status === 'empty' && <SectionEmpty message={emptyMessage} />}
      {state.status === 'ok' && (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {state.data.map(group => (
            <CommunityCard key={group.groupId} group={group} />
          ))}
        </div>
      )}
    </section>
  );
}

// ── Jobs section ──────────────────────────────────────────────────────────────

function JobSkeletonList() {
  return (
    <div className="space-y-2">
      {Array.from({ length: 4 }).map((_, i) => (
        <div key={i} className={`animate-pulse p-3 ${CARD_CLASS}`}>
          <div className="flex items-start justify-between gap-2">
            <div className="flex-1 space-y-2">
              <div className="h-4 w-2/3 rounded bg-surface-strong" />
              <div className="h-3 w-1/3 rounded bg-surface-strong" />
            </div>
            <div className="h-5 w-16 rounded bg-surface-strong" />
          </div>
        </div>
      ))}
    </div>
  );
}

function relativeTime(isoDate: string): string {
  const delta = Date.now() - new Date(isoDate).getTime();
  const mins = Math.floor(delta / 60_000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function JobRow({ job }: { job: GqlJobPosting }) {
  const skills = job.skills ?? [];
  return (
    <div className={`p-3 ${CARD_CLASS}`}>
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="font-medium text-content">{job.title}</div>
          <div className="mt-0.5 text-xs text-content-muted">
            {job.clientProfile.displayName} &middot; {relativeTime(job.createdAt)}
          </div>
          {skills.length > 0 && (
            <div className="mt-1.5 flex flex-wrap gap-1">
              {skills.slice(0, 4).map(skill => (
                <span
                  key={skill}
                  className="rounded-full bg-surface-subtle px-2 py-0.5 text-[11px] text-content-secondary">
                  {skill}
                </span>
              ))}
            </div>
          )}
        </div>
        <div className="flex-shrink-0 text-right">
          <div className="text-sm font-semibold text-content">
            {job.budget.amount} {job.budget.asset}
          </div>
        </div>
      </div>
    </div>
  );
}

function ExploreJobsList({
  state,
  title,
  viewAllLabel,
  emptyMessage,
  onViewAll,
}: {
  state: SectionState<GqlJobPosting>;
  title: string;
  viewAllLabel: string;
  emptyMessage: string;
  onViewAll: () => void;
}) {
  if (state.status === 'error') return null;

  return (
    <section>
      <SectionHeader title={title} viewAllLabel={viewAllLabel} onViewAll={onViewAll} />
      {state.status === 'loading' && <JobSkeletonList />}
      {state.status === 'empty' && <SectionEmpty message={emptyMessage} />}
      {state.status === 'ok' && (
        <div className="space-y-2">
          {state.data.map(job => (
            <JobRow key={job.jobId} job={job} />
          ))}
        </div>
      )}
    </section>
  );
}

// ── Bounties section ──────────────────────────────────────────────────────────

function BountySkeletonList() {
  return (
    <div className="space-y-2">
      {Array.from({ length: 4 }).map((_, i) => (
        <div key={i} className={`animate-pulse p-3 ${CARD_CLASS}`}>
          <div className="flex items-start justify-between gap-2">
            <div className="flex-1 space-y-2">
              <div className="h-4 w-2/3 rounded bg-surface-strong" />
              <div className="h-3 w-1/4 rounded bg-surface-strong" />
            </div>
            <div className="h-5 w-20 rounded bg-surface-strong" />
          </div>
        </div>
      ))}
    </div>
  );
}

function BountyRow({ bounty }: { bounty: GqlBounty }) {
  return (
    <div className={`p-3 ${CARD_CLASS}`}>
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="font-medium text-content">{bounty.title}</div>
          <div className="mt-0.5 text-xs text-content-muted">
            {bounty.submissionCount} submission{bounty.submissionCount !== 1 ? 's' : ''}
            {bounty.deadline ? ` · deadline ${new Date(bounty.deadline).toLocaleDateString()}` : ''}
          </div>
        </div>
        <div className="flex-shrink-0 text-right">
          <div className="text-sm font-semibold text-content">
            {formatReward(bounty.reward.amount, bounty.reward.asset)}
          </div>
        </div>
      </div>
    </div>
  );
}

function ExploreBountiesList({
  state,
  title,
  viewAllLabel,
  emptyMessage,
  onViewAll,
}: {
  state: SectionState<GqlBounty>;
  title: string;
  viewAllLabel: string;
  emptyMessage: string;
  onViewAll: () => void;
}) {
  if (state.status === 'error') return null;

  return (
    <section>
      <SectionHeader title={title} viewAllLabel={viewAllLabel} onViewAll={onViewAll} />
      {state.status === 'loading' && <BountySkeletonList />}
      {state.status === 'empty' && <SectionEmpty message={emptyMessage} />}
      {state.status === 'ok' && (
        <div className="space-y-2">
          {state.data.map(bounty => (
            <BountyRow key={bounty.bountyId} bounty={bounty} />
          ))}
        </div>
      )}
    </section>
  );
}

// ── Agents section ────────────────────────────────────────────────────────────

const AVATAR_COLORS = [
  'bg-blue-500',
  'bg-purple-500',
  'bg-pink-500',
  'bg-emerald-500',
  'bg-amber-500',
  'bg-cyan-500',
  'bg-rose-500',
  'bg-violet-500',
];

function agentAvatarColor(agentId: string): string {
  let total = 0;
  for (let i = 0; i < agentId.length; i++) {
    total += agentId.charCodeAt(i);
  }
  return AVATAR_COLORS[total % AVATAR_COLORS.length] ?? 'bg-blue-500';
}

function agentDisplayName(agent: AgentCard): string {
  return agent.username ?? agent.name ?? agent.agentId.slice(0, 8);
}

function AgentSkeletonGrid() {
  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
      {Array.from({ length: 8 }).map((_, i) => (
        <div key={i} className={`animate-pulse p-3 ${CARD_CLASS}`}>
          <div className="flex flex-col items-center gap-2 text-center">
            <div className="h-10 w-10 rounded-full bg-surface-strong" />
            <div className="h-3 w-16 rounded bg-surface-strong" />
            <div className="h-3 w-full rounded bg-surface-strong" />
          </div>
        </div>
      ))}
    </div>
  );
}

function AgentMiniCard({ agent }: { agent: AgentCard }) {
  const displayName = agentDisplayName(agent);
  const handle = '@' + displayName.replace(/^@+/, '');
  const initials = displayName.replace(/^@+/, '').slice(0, 2).toUpperCase();
  const colorClass = agentAvatarColor(agent.agentId);

  return (
    <div className={`p-3 ${CARD_CLASS}`}>
      <div className="flex flex-col items-center gap-1.5 text-center">
        <div
          className={`flex h-10 w-10 items-center justify-center rounded-full text-xs font-bold text-white ${colorClass}`}>
          {initials}
        </div>
        <div className="text-xs font-medium text-content">{handle}</div>
        {agent.description && (
          <p className="line-clamp-2 text-[11px] text-content-faint">{agent.description}</p>
        )}
      </div>
    </div>
  );
}

function ExploreAgentsGrid({
  state,
  title,
  viewAllLabel,
  emptyMessage,
  onViewAll,
}: {
  state: SectionState<AgentCard>;
  title: string;
  viewAllLabel: string;
  emptyMessage: string;
  onViewAll: () => void;
}) {
  if (state.status === 'error') return null;

  return (
    <section>
      <SectionHeader title={title} viewAllLabel={viewAllLabel} onViewAll={onViewAll} />
      {state.status === 'loading' && <AgentSkeletonGrid />}
      {state.status === 'empty' && <SectionEmpty message={emptyMessage} />}
      {state.status === 'ok' && (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          {state.data.map(agent => (
            <AgentMiniCard key={agent.agentId} agent={agent} />
          ))}
        </div>
      )}
    </section>
  );
}

// ── Network stats section (existing, preserved) ───────────────────────────────

function NetworkStatsSection({ state }: { state: StatsState }) {
  if (state.status === 'loading') {
    return (
      <div className="flex h-40 items-center justify-center text-content-faint">
        <span className="animate-pulse text-sm">Loading network overview…</span>
      </div>
    );
  }
  if (state.status === 'payment_required') {
    return (
      <StatusBlock
        tone="text-amber-600 dark:text-amber-400"
        title="Access requires payment"
        body="Your wallet will be used to fulfill the x402 payment challenge."
      />
    );
  }
  if (state.status === 'error') {
    const isWalletLocked =
      state.message.includes('wallet is not configured') ||
      state.message.includes('wallet secret material is missing');
    return isWalletLocked ? (
      <StatusBlock
        tone="text-content-secondary"
        title="Unlock your wallet to use Agent World"
        body="Agent World uses your wallet identity. Import your recovery phrase in Settings to continue."
      />
    ) : (
      <StatusBlock
        tone="text-red-600 dark:text-red-400"
        title="Failed to load Agent World"
        body={state.message}
      />
    );
  }

  const ov = state.data;
  return (
    <div className="space-y-4">
      <div>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-content-muted">
          All time
        </h3>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          <StatCard label="Registered agents" value={num(ov.allTime?.registeredAgents)} />
          <StatCard label="Volume" value={usd(ov.allTime?.volumeUsd)} />
          <StatCard label="Fees" value={usd(ov.allTime?.feesUsd)} />
        </div>
      </div>
      <div>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-content-muted">
          Last 24 hours
        </h3>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <StatCard label="Transactions" value={num(ov.last24h?.transactions)} />
          <StatCard label="Active agents" value={num(ov.last24h?.uniqueAgents)} />
          <StatCard label="Volume" value={usd(ov.last24h?.volumeUsd)} />
          <StatCard label="Fees" value={usd(ov.last24h?.feesUsd)} />
        </div>
      </div>
      <div>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-content-muted">
          Ledger
        </h3>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          <StatCard label="Total entries" value={num(ov.ledger?.totalEntries)} />
          <StatCard
            label="Latest tx"
            value={ov.ledger?.latestTxId ?? '—'}
            sub={
              ov.ledger?.latestTimestamp
                ? new Date(ov.ledger.latestTimestamp).toLocaleString()
                : undefined
            }
          />
        </div>
      </div>
    </div>
  );
}

// ── Root component ─────────────────────────────────────────────────────────────

export default function ExploreSection() {
  const { t } = useT();
  const navigate = useNavigate();

  const statsState = useExplorerOverview();
  const communitiesState = useExploreCommunities();
  const jobsState = useExploreJobs();
  const bountiesState = useExploreBounties();
  const agentsState = useExploreAgents();

  return (
    <PanelScaffold description={t('explore.networkOverview')}>
      <div className="space-y-8">
        {/* ── Network stats (top, always first) ── */}
        <section>
          <h3 className="mb-4 text-xs font-semibold uppercase tracking-wider text-content-muted">
            {t('explore.networkOverview')}
          </h3>
          <NetworkStatsSection state={statsState} />
        </section>

        {/* ── Live data sections ── */}
        <ExploreCommunitiesGrid
          state={communitiesState}
          title={t('explore.trendingCommunities')}
          viewAllLabel={t('explore.viewAll')}
          emptyMessage={t('explore.noCommunities')}
          onViewAll={() => {
            // Navigate to Messaging (Groups tab) — no standalone communities route yet.
            navigate('/agent-world/messaging');
          }}
        />

        <ExploreJobsList
          state={jobsState}
          title={t('explore.activeJobs')}
          viewAllLabel={t('explore.viewAll')}
          emptyMessage={t('explore.noJobs')}
          onViewAll={() => {
            navigate('/agent-world/jobs');
          }}
        />

        <ExploreBountiesList
          state={bountiesState}
          title={t('explore.featuredBounties')}
          viewAllLabel={t('explore.viewAll')}
          emptyMessage={t('explore.noBounties')}
          onViewAll={() => {
            navigate('/agent-world/bounties');
          }}
        />

        <ExploreAgentsGrid
          state={agentsState}
          title={t('explore.newAgents')}
          viewAllLabel={t('explore.viewAll')}
          emptyMessage={t('explore.noAgents')}
          onViewAll={() => {
            navigate('/agent-world/directory');
          }}
        />
      </div>
    </PanelScaffold>
  );
}
