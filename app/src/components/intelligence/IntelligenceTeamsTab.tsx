/**
 * IntelligenceTeamsTab — the agent-team coordination surface (#3374).
 *
 * A window onto the durable team ledger (PR1, #3546). It lists the teams an
 * agent has spawned, and — for a selected team — renders a {@link TeamHeader}
 * identity strip, the {@link TeamTaskBoard} of owned tasks, and an
 * always-visible {@link TeamActivityRail} of teammate messages. The board/rail
 * sit in a grid that collapses the rail under the board on narrow widths.
 *
 * PR4 makes it interactive on an active team: the lead/user can message a named
 * teammate (composer in the rail) and start a teammate live on its next
 * claimable task (the play affordance on each member chip), both wired through
 * {@link agentTeamApi}. A non-`started` outcome surfaces as a transient notice.
 *
 * Most users will see the EMPTY state first: a team only exists once an agent
 * spawns coordinated sub-agents, so "no teams yet" is the common first view and
 * is treated as a first-class surface rather than a blank panel.
 *
 * There is no socket event for team mutations yet (PR1 was storage + RPC only),
 * so freshness comes from a mount fetch + a light poll of the *selected* team
 * while it is still active + a manual refresh. Polling is scoped to the open
 * team (not the whole list) to bound cost, and stops when the team is closed.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { LuArrowLeft, LuRefreshCw, LuUsers } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type AgentTeam,
  agentTeamApi,
  type TeamMessage,
  type TeamView,
} from '../../services/api/agentTeamApi';
import Button from '../ui/Button';
import { TeamActivityRail } from './TeamActivityRail';
import { TeamHeader } from './TeamHeader';
import { TeamTaskBoard } from './TeamTaskBoard';

const log = debug('intelligence:teams');

/** Poll cadence for the open team while it is still active. */
const POLL_INTERVAL_MS = 5000;
/** Cap on messages fetched for the activity rail. */
const MESSAGE_LIMIT = 200;

export default function IntelligenceTeamsTab() {
  const { t } = useT();

  const [teams, setTeams] = useState<AgentTeam[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [view, setView] = useState<TeamView | null>(null);
  const [messages, setMessages] = useState<TeamMessage[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [sending, setSending] = useState(false);
  const [startingMemberId, setStartingMemberId] = useState<string | null>(null);
  // Transient feedback for a member-start outcome (blocked / already claimed /
  // nothing claimable) or a send/start error — distinct from the fatal `error`.
  const [actionNotice, setActionNotice] = useState<string | null>(null);
  const mountedRef = useRef(true);
  // Mirrors `view` for reads inside the poll interval without making the poll
  // effect depend on `view` (which would rebuild the interval every tick).
  const viewRef = useRef<TeamView | null>(null);
  // Mirrors `selectedId` so an in-flight `fetchDetail` can drop its result if
  // the selection changed (or cleared) while it was awaiting — a slower
  // detail fetch for an old team must not overwrite a newer selection.
  const selectedIdRef = useRef<string | null>(null);

  useEffect(() => {
    selectedIdRef.current = selectedId;
  }, [selectedId]);

  const fetchTeams = useCallback(async () => {
    log('fetchTeams: entry');
    setError(null);
    try {
      // Active teams only — `agent_team_list` returns closed teams too, and a
      // coordination board surfacing archived teams as primary entries is
      // noise. Closed-team history is a separate (future) surface.
      const rows = await agentTeamApi.list({ status: 'active' });
      if (!mountedRef.current) return;
      setTeams(rows);
      log('fetchTeams: done teams=%d', rows.length);
      // Auto-select when there is exactly one team — the common single-team
      // case lands straight on the board instead of a one-row list.
      if (rows.length === 1) setSelectedId(prev => prev ?? rows[0].id);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('fetchTeams: error %s', msg);
      if (mountedRef.current) setError(msg);
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  const fetchDetail = useCallback(async (teamId: string) => {
    log('fetchDetail: entry team=%s', teamId);
    const [nextView, nextMessages] = await Promise.all([
      agentTeamApi.get(teamId),
      agentTeamApi.listMessages(teamId, MESSAGE_LIMIT),
    ]);
    // Drop a stale response: unmounted, or the selection moved on while we
    // were awaiting (guards the slow-fetch-overwrites-newer-team race).
    if (!mountedRef.current || selectedIdRef.current !== teamId) return;
    setView(nextView);
    viewRef.current = nextView;
    setMessages(nextMessages);
    log('fetchDetail: done found=%o messages=%d', nextView != null, nextMessages.length);
  }, []);

  // Deselect a team: clears the detail synchronously from the event handler
  // (not an effect) so going "back to list" never flashes a stale board.
  const deselect = useCallback(() => {
    setSelectedId(null);
    setView(null);
    viewRef.current = null;
    setMessages([]);
    // Reset transient per-team feedback so a notice from the team we just left
    // doesn't leak onto the next team opened.
    setActionNotice(null);
    setStartingMemberId(null);
  }, []);

  // Select a team, clearing any transient notice from a previously viewed team
  // so stale feedback doesn't carry across navigation.
  const selectTeam = useCallback((teamId: string) => {
    setActionNotice(null);
    setStartingMemberId(null);
    setSelectedId(teamId);
  }, []);

  // Mount fetch of the team list (mirrors IntelligenceAgentWorkTab's 0ms
  // setTimeout so the first paint shows the loading state).
  useEffect(() => {
    mountedRef.current = true;
    const handle = window.setTimeout(() => void fetchTeams(), 0);
    return () => {
      window.clearTimeout(handle);
      mountedRef.current = false;
    };
  }, [fetchTeams]);

  // Load + poll the selected team's detail while it is active. Keyed on
  // `selectedId` only — the closed-team check reads `viewRef` so the interval
  // isn't torn down and rebuilt on every poll response.
  useEffect(() => {
    if (!selectedId) return;
    void fetchDetail(selectedId).catch(err => {
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    });
    const interval = window.setInterval(() => {
      // Stop polling once the team is closed; the board won't change further.
      if (viewRef.current?.team.status === 'closed') return;
      void fetchDetail(selectedId).catch(() => {
        /* transient poll failure — keep the last good view */
      });
    }, POLL_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [selectedId, fetchDetail]);

  const refresh = useCallback(async () => {
    // Clear any prior error before retrying. Without this a successful retry
    // after a `fetchDetail` failure would refresh `view`/`messages` but leave
    // `error` set, so the error-branch render short-circuits and the user is
    // stuck on the banner forever. `fetchTeams` self-clears; `fetchDetail`
    // deliberately doesn't (so silent polls don't wipe a visible error), so the
    // clear belongs here on the explicit user-driven retry.
    setError(null);
    setRefreshing(true);
    try {
      if (selectedId) await fetchDetail(selectedId);
      else await fetchTeams();
    } catch (err) {
      if (mountedRef.current) setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (mountedRef.current) setRefreshing(false);
    }
  }, [selectedId, fetchDetail, fetchTeams]);

  // Send a message as the lead (fromMemberId omitted) to a teammate or the
  // whole team, then refresh so the new message appears in the rail.
  const handleSend = useCallback(
    async (toMemberId: string | null, content: string) => {
      if (!selectedId) return;
      setSending(true);
      setActionNotice(null);
      try {
        await agentTeamApi.messageMember({
          teamId: selectedId,
          toMemberId: toMemberId ?? undefined,
          content,
        });
        await fetchDetail(selectedId);
      } catch (err) {
        if (mountedRef.current) {
          setActionNotice(err instanceof Error ? err.message : String(err));
        }
        // Re-throw so the composer's onSend rejects and keeps the unsent draft.
        // Swallowing here would resolve handleSend as success and TeamActivityRail
        // would clear the draft text on a failed send.
        throw err;
      } finally {
        if (mountedRef.current) setSending(false);
      }
    },
    [selectedId, fetchDetail]
  );

  // Start a live run for a member (auto-picks the member's next claimable task).
  // A non-`started` outcome is surfaced as a transient notice; either way we
  // refresh so the board/roster reflect the new state.
  const handleStartMember = useCallback(
    async (memberId: string) => {
      if (!selectedId) return;
      setStartingMemberId(memberId);
      setActionNotice(null);
      try {
        const outcome = await agentTeamApi.startMember({ teamId: selectedId, memberId });
        if (outcome.kind === 'blocked') {
          // Name the unmet dependency ids so the user knows what to finish
          // first (t() does not interpolate — compose at the call site).
          const base = t('intelligence.teams.action.blocked');
          setActionNotice(outcome.unmet.length ? `${base}: ${outcome.unmet.join(', ')}` : base);
        } else if (outcome.kind !== 'started') {
          setActionNotice(t(`intelligence.teams.action.${outcome.kind}`));
        }
        await fetchDetail(selectedId);
      } catch (err) {
        if (mountedRef.current) {
          setActionNotice(err instanceof Error ? err.message : String(err));
        }
      } finally {
        if (mountedRef.current) setStartingMemberId(null);
      }
    },
    [selectedId, fetchDetail, t]
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center py-10 text-content-faint">
        <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
        <span className="text-sm">{t('intelligence.teams.loading')}</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="space-y-3">
        <div className="rounded-xl border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
          {t('intelligence.teams.failedToLoad')}: {error}
        </div>
        <RefreshButton refreshing={refreshing} onClick={() => void refresh()} t={t} />
      </div>
    );
  }

  if (teams.length === 0) {
    return (
      <div className="space-y-4">
        <p className="text-xs text-content-faint">{t('intelligence.teams.subtitle')}</p>
        <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-line py-12 text-center">
          <LuUsers className="h-6 w-6 text-content-faint dark:text-neutral-600" />
          <p className="text-sm text-content-muted">{t('intelligence.teams.empty')}</p>
          <p className="max-w-sm text-xs text-content-faint">{t('intelligence.teams.emptyHint')}</p>
        </div>
      </div>
    );
  }

  // Detail view — selected team's board + activity rail.
  if (selectedId && view) {
    return (
      <div className="space-y-3">
        <div className="flex items-center justify-between gap-2">
          {teams.length > 1 ? (
            <Button
              variant="secondary"
              size="xs"
              onClick={deselect}
              leadingIcon={<LuArrowLeft className="h-3 w-3" />}
              className="gap-1 text-[11px]">
              {t('intelligence.teams.backToList')}
            </Button>
          ) : (
            <span />
          )}
          <RefreshButton refreshing={refreshing} onClick={() => void refresh()} t={t} />
        </div>

        <TeamHeader
          team={view.team}
          members={view.members}
          taskCount={view.tasks.length}
          onStartMember={
            view.team.status === 'active' ? memberId => void handleStartMember(memberId) : undefined
          }
          startingMemberId={startingMemberId}
        />

        {actionNotice && (
          <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-[11px] text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300">
            {actionNotice}
          </div>
        )}

        <div className="grid grid-cols-1 gap-3 xl:grid-cols-[1fr_300px]">
          <TeamTaskBoard tasks={view.tasks} members={view.members} />
          <TeamActivityRail
            messages={messages}
            members={view.members}
            onSend={view.team.status === 'active' ? handleSend : undefined}
            sending={sending}
          />
        </div>
      </div>
    );
  }

  // List view — pick a team.
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs text-content-faint">{t('intelligence.teams.subtitle')}</p>
        <RefreshButton refreshing={refreshing} onClick={() => void refresh()} t={t} />
      </div>
      <ul className="divide-y divide-line-subtle overflow-hidden rounded-xl border border-line bg-surface dark:divide-neutral-800">
        {teams.map(team => (
          <li key={team.id}>
            <button
              type="button"
              onClick={() => selectTeam(team.id)}
              className="flex w-full items-center justify-between gap-3 p-3 text-left hover:bg-surface-hover">
              <div className="min-w-0">
                <p className="truncate text-sm font-medium text-content">
                  {team.summary?.trim() || team.leadAgentId}
                </p>
                <p className="truncate text-[11px] text-content-muted">
                  {t('intelligence.teams.header.lead')}{' '}
                  <span className="font-mono">{team.leadAgentId}</span>
                </p>
              </div>
              <span
                className={`flex-none rounded-md px-1.5 py-0.5 text-[10px] ${
                  team.status === 'active'
                    ? 'bg-sage-50 text-sage-700 dark:bg-sage-500/10 dark:text-sage-300'
                    : 'bg-surface-subtle text-content-muted'
                }`}>
                {team.status === 'active'
                  ? t('intelligence.teams.status.active')
                  : t('intelligence.teams.status.closed')}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function RefreshButton({
  refreshing,
  onClick,
  t,
}: {
  refreshing: boolean;
  onClick: () => void;
  t: (key: string) => string;
}) {
  return (
    <Button
      variant="secondary"
      size="xs"
      disabled={refreshing}
      onClick={onClick}
      leadingIcon={<LuRefreshCw className={`h-3 w-3 ${refreshing ? 'animate-spin' : ''}`} />}
      className="gap-1 text-[11px]">
      {t('intelligence.teams.refresh')}
    </Button>
  );
}
