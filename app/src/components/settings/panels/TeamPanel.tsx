import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { teamApi } from '../../../services/api/teamApi';
import { CoreRpcError } from '../../../services/coreRpcClient';
import type { TeamWithRole } from '../../../types/team';
import { sanitizeError } from '../../../utils/sanitize';
import { CenteredLoadingState, ErrorBanner } from '../../ui';
import Button from '../../ui/Button';
import { SettingsBadge, SettingsSection, SettingsTextField } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';
import { teamErrorMessage } from './teamErrorMessage';

const log = debug('core-rpc:error');

const TeamPanel = () => {
  const { t } = useT();
  const { navigateToTeamManagement } = useSettingsNavigation();
  const { snapshot, teams, refresh, refreshTeams } = useCoreState();
  const user = snapshot.currentUser;

  const [joinCode, setJoinCode] = useState('');
  const [isLoading, setIsLoading] = useState(false);
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
      setError(teamErrorMessage(err, t('team.invalidInviteCode')));
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
      setError(teamErrorMessage(err, t('team.failedToSwitch')));
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
      setError(teamErrorMessage(err, t('team.failedToLeave')));
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

    const variantMap: Record<string, 'primary' | 'warning' | 'neutral'> = {
      ADMIN: 'primary',
      BILLING_MANAGER: 'warning',
      MEMBER: 'neutral',
    };

    return (
      <SettingsBadge variant={variantMap[normalizedRole] ?? 'neutral'}>{roleLabel}</SettingsBadge>
    );
  };

  const planBadge = (plan: string) => {
    const variantMap: Record<string, 'primary' | 'success' | 'neutral'> = {
      PRO: 'primary',
      BASIC: 'primary',
      FREE: 'neutral',
    };
    return <SettingsBadge variant={variantMap[plan] ?? 'neutral'}>{plan}</SettingsBadge>;
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
            : 'border-line bg-surface hover:bg-surface-hover'
        }`}>
        <div className="flex items-center gap-3 min-w-0 flex-1">
          <div className="w-9 h-9 rounded-lg bg-surface-subtle flex items-center justify-center flex-shrink-0">
            <span className="text-sm font-semibold text-content-secondary">
              {team.name.charAt(0).toUpperCase()}
            </span>
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-sm font-medium text-content truncate">{team.name}</span>
              {roleBadge(role, team.createdBy)}
              {planBadge(team.subscription.plan)}
              {isActive && <SettingsBadge variant="success">{t('team.active')}</SettingsBadge>}
            </div>
            {team.isPersonal && (
              <p className="text-xs text-content-muted mt-0.5">{t('team.personalTeam')}</p>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2 flex-shrink-0">
          {canManage && (
            <Button
              type="button"
              variant="secondary"
              size="xs"
              onClick={() => navigateToTeamManagement(team._id)}>
              {t('team.manageTeam')}
            </Button>
          )}
          {!isActive && (
            <Button
              type="button"
              variant="secondary"
              size="xs"
              onClick={() => handleSwitchTeam(team._id)}
              disabled={isSwitching === team._id}>
              {isSwitching === team._id ? t('team.switching') : t('team.switch')}
            </Button>
          )}
          {canLeave && (
            <Button
              type="button"
              variant="tertiary"
              size="xs"
              onClick={() => handleLeaveTeam(entry)}
              disabled={isLeaving === team._id}
              className="text-amber-700 dark:text-amber-300 hover:bg-amber-50 dark:hover:bg-amber-500/10">
              {isLeaving === team._id ? t('team.leaving') : t('team.leave')}
            </Button>
          )}
        </div>
      </div>
    );
  };

  return (
    <SettingsPanel description={t('pages.settings.account.teamDesc')}>
      {error && <ErrorBanner message={error} />}

      {isLoading && teams.length === 0 && <CenteredLoadingState />}

      {teams.length > 0 && (
        <SettingsSection title={`${t('team.yourTeams')} (${teams.length})`}>
          <div className="p-3 space-y-2">
            {teams.map(entry => (
              <TeamRow key={entry.team._id} entry={entry} />
            ))}
          </div>
        </SettingsSection>
      )}

      <SettingsSection title={t('team.joinExistingTeam')}>
        <p className="px-4 pt-3 text-xs text-content-muted">{t('team.personalAutoCreatedNote')}</p>
        <div className="flex gap-2 px-4 py-3">
          <SettingsTextField
            mono
            className="flex-1"
            value={joinCode}
            onChange={e => setJoinCode(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && void handleJoinTeam()}
            placeholder={t('team.inviteCode')}
            aria-label={t('team.inviteCode')}
            inputSize="sm"
          />
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => void handleJoinTeam()}
            disabled={isJoining || !joinCode.trim()}>
            {isJoining ? t('team.joining') : t('team.join')}
          </Button>
        </div>
      </SettingsSection>

      {teamToLeave && (
        <div className="fixed inset-0 bg-neutral-900/50 flex items-center justify-center z-50 p-4">
          <div className="bg-surface rounded-2xl p-6 w-full max-w-md border border-line">
            <h3 className="text-sm font-semibold text-content mb-4">{t('team.leaveTeam')}</h3>

            {error && (
              <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                <p className="text-xs text-coral-400">{error}</p>
              </div>
            )}

            <div className="space-y-4">
              <div className="text-sm text-content-muted">
                <p>
                  {t('team.confirmLeave')}{' '}
                  <strong className="text-content">{teamToLeave.team.name}</strong>?
                </p>
                <p className="mt-2 text-amber-400">{t('team.leaveWarning')}</p>
              </div>

              <div className="flex gap-2 pt-2">
                <Button
                  type="button"
                  variant="secondary"
                  size="md"
                  className="flex-1"
                  onClick={() => setTeamToLeave(null)}
                  disabled={isLeaving === teamToLeave.team._id}>
                  {t('common.cancel')}
                </Button>
                <Button
                  type="button"
                  variant="tertiary"
                  size="md"
                  className="flex-1 bg-amber-500 hover:bg-amber-600 text-content-inverted border-0 dark:bg-amber-500 dark:hover:bg-amber-600"
                  onClick={() => void confirmLeaveTeam()}
                  disabled={isLeaving === teamToLeave.team._id}>
                  {isLeaving === teamToLeave.team._id ? t('team.leaving') : t('team.leaveTeam')}
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}
    </SettingsPanel>
  );
};

export default TeamPanel;
