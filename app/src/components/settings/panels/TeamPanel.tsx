import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { teamApi } from '../../../services/api/teamApi';
import { CoreRpcError } from '../../../services/coreRpcClient';
import type { TeamWithRole } from '../../../types/team';
import { sanitizeError } from '../../../utils/sanitize';
import { CenteredLoadingState, ErrorBanner } from '../../ui';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = debug('core-rpc:error');

const TeamPanel = () => {
  const { t } = useT();
  const { navigateBack, navigateToTeamManagement, breadcrumbs } = useSettingsNavigation();
  const { snapshot, teams, refresh, refreshTeams } = useCoreState();
  const user = snapshot.currentUser;

  const [newTeamName, setNewTeamName] = useState('');
  const [joinCode, setJoinCode] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [isJoining, setIsJoining] = useState(false);
  const [isSwitching, setIsSwitching] = useState<string | null>(null);
  const [isLeaving, setIsLeaving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [teamToLeave, setTeamToLeave] = useState<TeamWithRole | null>(null);

  const activeTeamId = user?.activeTeamId;

  const refreshTeamsWithLoading = useCallback(async () => {
    setIsLoading(true);
    try {
      await refreshTeams();
    } catch (err) {
      // Bootstrap-time `team_list_teams` failures (cold core boot, backend
      // 504, local AbortController hit `CORE_RPC_TIMEOUT_MS`) used to leak
      // as unhandled promise rejections via the `void` in the useEffect
      // below, polluting Sentry as OPENHUMAN-REACT-15/11. The next visible
      // user action retries, so swallow silently for transient kinds.
      const kind = err instanceof CoreRpcError ? err.kind : 'unknown';
      log('refreshTeams failed in TeamPanel (kind=%s): %O', kind, sanitizeError(err));
    } finally {
      setIsLoading(false);
    }
  }, [refreshTeams]);

  useEffect(() => {
    // `refreshTeamsWithLoading` already absorbs rejections internally, but
    // keep the `.catch()` as a belt-and-suspenders guard so a future refactor
    // that re-throws cannot regress the unhandled-rejection family.
    refreshTeamsWithLoading().catch(err => {
      log('refreshTeamsWithLoading rethrew unexpectedly: %O', sanitizeError(err));
    });
  }, [refreshTeamsWithLoading]);

  const handleCreateTeam = async () => {
    const name = newTeamName.trim();
    if (!name) return;
    setIsCreating(true);
    setError(null);
    try {
      await teamApi.createTeam(name);
      setNewTeamName('');
      await refreshTeamsWithLoading();
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : t('team.failedToCreate')
      );
    } finally {
      setIsCreating(false);
    }
  };

  const handleJoinTeam = async () => {
    const code = joinCode.trim();
    if (!code) return;
    setIsJoining(true);
    setError(null);
    try {
      await teamApi.joinTeam(code);
      setJoinCode('');
      await Promise.all([refresh(), refreshTeamsWithLoading()]);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : t('team.invalidInviteCode')
      );
    } finally {
      setIsJoining(false);
    }
  };

  const handleSwitchTeam = async (teamId: string) => {
    if (teamId === activeTeamId) return;
    setIsSwitching(teamId);
    setError(null);
    try {
      await teamApi.switchTeam(teamId);
      await Promise.all([refresh(), refreshTeamsWithLoading()]);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : t('team.failedToSwitch')
      );
    } finally {
      setIsSwitching(null);
    }
  };

  const handleLeaveTeam = (teamEntry: TeamWithRole) => {
    setTeamToLeave(teamEntry);
  };

  const confirmLeaveTeam = async () => {
    if (!teamToLeave) return;

    setIsLeaving(teamToLeave.team._id);
    setError(null);

    try {
      await teamApi.leaveTeam(teamToLeave.team._id);
      await Promise.all([refresh(), refreshTeamsWithLoading()]);
      setTeamToLeave(null);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : t('team.failedToLeave')
      );
    } finally {
      setIsLeaving(null);
    }
  };

  const roleBadge = (role: string, teamCreatedBy?: string) => {
    const normalizedRole = role.toUpperCase();
    const isOwner = normalizedRole === 'ADMIN' && teamCreatedBy === user?._id;

    const roleLabel = isOwner
      ? t('team.role.owner')
      : normalizedRole === 'ADMIN'
        ? t('team.role.admin')
        : normalizedRole === 'BILLING_MANAGER'
          ? t('team.role.billingManager')
          : t('team.role.member');

    const colors: Record<string, string> = {
      ADMIN: 'bg-primary-500/20 text-primary-400 border-primary-500/30',
      BILLING_MANAGER: 'bg-amber-500/20 text-amber-400 border-amber-500/30',
      MEMBER:
        'bg-stone-50 dark:bg-neutral-800/60 text-stone-400 dark:text-neutral-500 border-stone-500/30',
    };

    return (
      <span
        className={`px-1.5 py-0.5 text-[10px] font-medium rounded-full border ${colors[normalizedRole] ?? colors.MEMBER}`}>
        {roleLabel}
      </span>
    );
  };

  const planBadge = (plan: string) => {
    const colors: Record<string, string> = {
      PRO: 'bg-lavender-500/20 text-lavender-400 border-lavender-500/30',
      BASIC: 'bg-primary-500/20 text-primary-400 border-primary-500/30',
      FREE: 'bg-stone-50 dark:bg-neutral-800/60 text-stone-400 dark:text-neutral-500 border-stone-500/30',
    };
    return (
      <span
        className={`px-1.5 py-0.5 text-[10px] font-medium rounded-full border ${colors[plan] ?? colors.FREE}`}>
        {plan}
      </span>
    );
  };

  const TeamRow = ({ entry }: { entry: TeamWithRole }) => {
    const { team, role } = entry;
    const isActive = team._id === activeTeamId;
    const normalizedRole = role.toUpperCase();
    const canLeave = !team.isPersonal && normalizedRole !== 'ADMIN';
    const canManage = normalizedRole === 'ADMIN' && !team.isPersonal;

    return (
      <div
        className={`flex items-center justify-between p-3 rounded-xl border transition-all ${
          isActive
            ? 'border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10'
            : 'border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60 dark:hover:bg-neutral-800/60'
        }`}>
        <div className="flex items-center gap-3 min-w-0 flex-1">
          <div className="w-9 h-9 rounded-lg bg-stone-100 dark:bg-neutral-800 flex items-center justify-center flex-shrink-0">
            <span className="text-sm font-semibold text-stone-600 dark:text-neutral-300">
              {team.name.charAt(0).toUpperCase()}
            </span>
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-sm font-medium text-stone-900 dark:text-neutral-100 truncate">
                {team.name}
              </span>
              {roleBadge(role, team.createdBy)}
              {planBadge(team.subscription.plan)}
              {isActive && (
                <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-sage-500/20 text-sage-400 border border-sage-500/30">
                  {t('team.active')}
                </span>
              )}
            </div>
            {team.isPersonal && (
              <p className="text-xs text-stone-400 dark:text-neutral-500 mt-0.5">
                {t('team.personalTeam')}
              </p>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2 flex-shrink-0">
          {canManage && (
            <button
              onClick={() => navigateToTeamManagement(team._id)}
              className="px-2.5 py-1 text-xs font-medium rounded-lg bg-primary-50 dark:bg-primary-500/10 hover:bg-primary-100 dark:bg-primary-500/20 text-primary-600 dark:text-primary-300 transition-colors">
              {t('team.manageTeam')}
            </button>
          )}
          {!isActive && (
            <button
              onClick={() => handleSwitchTeam(team._id)}
              disabled={isSwitching === team._id}
              className="px-2.5 py-1 text-xs font-medium rounded-lg bg-stone-100 dark:bg-neutral-800 hover:bg-stone-200 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300 transition-colors disabled:opacity-50">
              {isSwitching === team._id ? t('team.switching') : t('team.switch')}
            </button>
          )}
          {canLeave && (
            <button
              onClick={() => handleLeaveTeam(entry)}
              disabled={isLeaving === team._id}
              className="px-2.5 py-1 text-xs font-medium rounded-lg text-amber-700 dark:text-amber-300 hover:bg-amber-50 dark:bg-amber-500/10 transition-colors disabled:opacity-50">
              {isLeaving === team._id ? t('team.leaving') : t('team.leave')}
            </button>
          )}
        </div>
      </div>
    );
  };

  return (
    <div>
      <SettingsHeader
        title={t('settings.account.team')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4 space-y-4">
          {error && <ErrorBanner message={error} />}

          {isLoading && teams.length === 0 && <CenteredLoadingState />}

          {teams.length > 0 && (
            <div className="space-y-3">
              <h3 className="text-xs font-medium text-stone-500 dark:text-neutral-400 uppercase tracking-wider px-1">
                {t('team.yourTeams')} ({teams.length})
              </h3>
              <div className="space-y-2">
                {teams.map(entry => (
                  <TeamRow key={entry.team._id} entry={entry} />
                ))}
              </div>
            </div>
          )}

          <div className="space-y-4 border-t border-stone-200 dark:border-neutral-800 pt-4">
            <div className="space-y-2">
              <h3 className="text-xs font-medium text-stone-500 dark:text-neutral-400 uppercase tracking-wider px-1">
                {t('team.createNewTeam')}
              </h3>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={newTeamName}
                  onChange={e => setNewTeamName(e.target.value)}
                  onKeyDown={e => e.key === 'Enter' && handleCreateTeam()}
                  placeholder={t('team.teamName')}
                  className="flex-1 px-3 py-2 text-sm bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 rounded-xl text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:outline-none focus:border-primary-500/50"
                />
                <button
                  onClick={handleCreateTeam}
                  disabled={isCreating || !newTeamName.trim()}
                  className="px-4 py-2 text-xs font-medium rounded-xl bg-primary-500 hover:bg-primary-600 text-white transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
                  {isCreating ? t('team.creating') : t('common.create')}
                </button>
              </div>
            </div>

            <div className="space-y-2">
              <h3 className="text-xs font-medium text-stone-500 dark:text-neutral-400 uppercase tracking-wider px-1">
                {t('team.joinExistingTeam')}
              </h3>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={joinCode}
                  onChange={e => setJoinCode(e.target.value)}
                  onKeyDown={e => e.key === 'Enter' && handleJoinTeam()}
                  placeholder={t('team.inviteCode')}
                  className="flex-1 px-3 py-2 text-sm bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 rounded-xl text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:outline-none focus:border-primary-500/50 font-mono"
                />
                <button
                  onClick={handleJoinTeam}
                  disabled={isJoining || !joinCode.trim()}
                  className="px-4 py-2 text-xs font-medium rounded-xl bg-stone-100 dark:bg-neutral-800 hover:bg-stone-200 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300 transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
                  {isJoining ? t('team.joining') : t('team.join')}
                </button>
              </div>
            </div>
          </div>

          {teamToLeave && (
            <div className="fixed inset-0 bg-stone-900/50 flex items-center justify-center z-50 p-4">
              <div className="bg-white dark:bg-neutral-900 rounded-2xl p-6 w-full max-w-md border border-stone-200 dark:border-neutral-800">
                <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100 mb-4">
                  {t('team.leaveTeam')}
                </h3>

                {error && (
                  <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                    <p className="text-xs text-coral-400">{error}</p>
                  </div>
                )}

                <div className="space-y-4">
                  <div className="text-sm text-stone-500 dark:text-neutral-400">
                    <p>
                      {t('team.confirmLeave')}{' '}
                      <strong className="text-stone-900 dark:text-neutral-100">
                        {teamToLeave.team.name}
                      </strong>
                      ?
                    </p>
                    <p className="mt-2 text-amber-400">{t('team.leaveWarning')}</p>
                  </div>

                  <div className="flex gap-2 pt-2">
                    <button
                      onClick={() => setTeamToLeave(null)}
                      disabled={isLeaving === teamToLeave.team._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-stone-100 dark:bg-neutral-800 hover:bg-stone-200 dark:bg-neutral-800 text-stone-600 dark:text-neutral-300 transition-colors disabled:opacity-50">
                      {t('common.cancel')}
                    </button>
                    <button
                      onClick={confirmLeaveTeam}
                      disabled={isLeaving === teamToLeave.team._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-amber-500 hover:bg-amber-600 text-white transition-colors disabled:opacity-50">
                      {isLeaving === teamToLeave.team._id ? t('team.leaving') : t('team.leaveTeam')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default TeamPanel;
