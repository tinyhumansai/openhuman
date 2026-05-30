import debug from 'debug';
import { useEffect, useState } from 'react';
import { useLocation, useParams } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { teamApi } from '../../../services/api/teamApi';
import type { TeamMember, TeamRole } from '../../../types/team';
import { sanitizeError } from '../../../utils/sanitize';
import { CenteredLoadingState, ErrorBanner, InlineLoadingStatus } from '../../ui';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = debug('core-rpc:error');

const ROLES: TeamRole[] = ['ADMIN', 'BILLING_MANAGER', 'MEMBER'];

const TeamMembersPanel = () => {
  const { t } = useT();
  const { teamId } = useParams<{ teamId: string }>();
  const location = useLocation();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { snapshot, teams, teamMembersById, refreshTeamMembers } = useCoreState();
  const user = snapshot.currentUser;

  // Check if we're in team management context (has teamId in URL)
  const isInManagementContext = location.pathname.includes('/team/manage/');
  const currentTeamId = isInManagementContext ? teamId : user?.activeTeamId;
  const currentTeam = teams.find(t => t.team._id === currentTeamId);
  const isAdmin = currentTeam?.role.toUpperCase() === 'ADMIN';
  const members = currentTeamId ? (teamMembersById[currentTeamId] ?? []) : [];

  const [removingId, setRemovingId] = useState<string | null>(null);
  const [changingRoleId, setChangingRoleId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoadingMembers, setIsLoadingMembers] = useState(false);

  // Confirmation modals state
  const [memberToRemove, setMemberToRemove] = useState<TeamMember | null>(null);
  const [roleChangeConfirmation, setRoleChangeConfirmation] = useState<{
    member: TeamMember;
    newRole: TeamRole;
    oldRole: TeamRole;
  } | null>(null);

  useEffect(() => {
    if (!currentTeamId) return;
    setIsLoadingMembers(true);
    // `.finally()` alone left this as `void promise(...)`, so any rejection
    // (cold core boot, backend 504, local AbortController timeout) became an
    // unhandled rejection → OPENHUMAN-REACT-10. Swallow into a logged
    // breadcrumb; the user can retry by navigating away and back.
    refreshTeamMembers(currentTeamId)
      .catch(err => {
        log('refreshTeamMembers failed in TeamMembersPanel: %O', sanitizeError(err));
      })
      .finally(() => setIsLoadingMembers(false));
  }, [currentTeamId, refreshTeamMembers]);

  const handleChangeRole = (member: TeamMember, newRole: TeamRole) => {
    if (!currentTeamId || member.role === newRole) return;

    // Show confirmation modal for role changes
    setRoleChangeConfirmation({ member, newRole, oldRole: member.role as TeamRole });
  };

  const confirmChangeRole = async () => {
    if (!roleChangeConfirmation || !currentTeamId) return;

    const { member, newRole } = roleChangeConfirmation;
    setChangingRoleId(member._id);
    setError(null);

    try {
      await teamApi.changeMemberRole(currentTeamId, member.user._id, newRole);
      await refreshTeamMembers(currentTeamId);
      setRoleChangeConfirmation(null);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : t('team.failedChangeRole')
      );
    } finally {
      setChangingRoleId(null);
    }
  };

  const handleRemoveMember = (member: TeamMember) => {
    // Show confirmation modal for removing members
    setMemberToRemove(member);
  };

  const confirmRemoveMember = async () => {
    if (!memberToRemove || !currentTeamId) return;

    setRemovingId(memberToRemove._id);
    setError(null);

    try {
      await teamApi.removeMember(currentTeamId, memberToRemove.user._id);
      await refreshTeamMembers(currentTeamId);
      setMemberToRemove(null);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : t('team.failedRemoveMember')
      );
    } finally {
      setRemovingId(null);
    }
  };

  const displayName = (m: TeamMember) => {
    const parts = [m.user.firstName, m.user.lastName].filter(Boolean);
    if (parts.length) return parts.join(' ');
    if (m.user.username) return m.user.username;
    return 'Unknown';
  };

  const isCurrentUser = (m: TeamMember) => m.user._id === user?._id;

  const roleBadgeColor: Record<string, string> = {
    ADMIN: 'bg-primary-500/20 text-primary-400 border-primary-500/30',
    BILLING_MANAGER: 'bg-amber-500/20 text-amber-400 border-amber-500/30',
    MEMBER:
      'bg-stone-50 dark:bg-neutral-800/60 text-stone-400 dark:text-neutral-500 border-stone-500/30',
  };

  return (
    <div>
      <SettingsHeader
        title={t('team.members')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4 space-y-4">
          {error && <ErrorBanner message={error} />}

          {/* Refreshing indicator - only when loading and has existing data */}
          {isLoadingMembers && members.length > 0 && (
            <InlineLoadingStatus label={t('team.refreshingMembers')} />
          )}

          {/* Member count */}
          <p className="text-xs text-stone-500 dark:text-neutral-400 px-1">
            {t(members.length === 1 ? 'team.memberCount' : 'team.memberCountPlural').replace(
              '{count}',
              String(members.length)
            )}
          </p>

          {/* Full loading state - only when loading and no existing data */}
          {isLoadingMembers && members.length === 0 ? (
            <CenteredLoadingState label={t('team.loadingMembers')} />
          ) : (
            <div className="space-y-2">
              {members.map(member => (
                <div
                  key={member._id}
                  className="flex items-center justify-between p-3 rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900">
                  <div className="flex items-center gap-3 min-w-0">
                    {/* Avatar */}
                    <div className="w-8 h-8 rounded-full bg-stone-700/60 flex items-center justify-center flex-shrink-0">
                      <span className="text-xs font-semibold text-white">
                        {displayName(member).charAt(0).toUpperCase()}
                      </span>
                    </div>
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium text-stone-900 dark:text-neutral-100 truncate">
                          {displayName(member)}
                        </span>
                        {isCurrentUser(member) && (
                          <span className="text-[10px] text-stone-500 dark:text-neutral-400">
                            {t('team.you')}
                          </span>
                        )}
                      </div>
                      {member.user.username && (
                        <p className="text-xs text-stone-500 dark:text-neutral-400 truncate">
                          @{member.user.username}
                        </p>
                      )}
                    </div>
                  </div>

                  <div className="flex items-center gap-2 flex-shrink-0">
                    {/* Role badge / dropdown */}
                    {isAdmin && !isCurrentUser(member) ? (
                      <select
                        value={member.role.toUpperCase()}
                        onChange={e => handleChangeRole(member, e.target.value as TeamRole)}
                        disabled={changingRoleId === member._id}
                        className="px-2 py-1 text-[10px] font-medium rounded-full border bg-white dark:bg-neutral-900 text-stone-700 dark:text-neutral-200 border-stone-300 dark:border-neutral-700 focus:outline-none focus:border-primary-500/50 disabled:opacity-50">
                        {ROLES.map(r => (
                          <option key={r} value={r}>
                            {r}
                          </option>
                        ))}
                      </select>
                    ) : (
                      <span
                        className={`px-1.5 py-0.5 text-[10px] font-medium rounded-full border ${roleBadgeColor[member.role.toUpperCase()] ?? roleBadgeColor.MEMBER}`}>
                        {member.role.toUpperCase()}
                      </span>
                    )}

                    {/* Remove button (admin only, not self) */}
                    {isAdmin && !isCurrentUser(member) && (
                      <button
                        onClick={() => handleRemoveMember(member)}
                        disabled={removingId === member._id}
                        className="p-1 rounded-lg text-stone-500 dark:text-neutral-400 hover:text-coral-400 hover:bg-coral-500/10 transition-colors disabled:opacity-50"
                        aria-label={t('team.removeAria').replace('{name}', displayName(member))}>
                        <svg
                          className="w-4 h-4"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M6 18L18 6M6 6l12 12"
                          />
                        </svg>
                      </button>
                    )}
                  </div>
                </div>
              ))}

              {members.length === 0 && !isLoadingMembers && (
                <div className="text-center py-8">
                  <p className="text-sm text-stone-500 dark:text-neutral-400">
                    {t('team.noMembers')}
                  </p>
                </div>
              )}
            </div>
          )}

          {/* Remove Member Confirmation Modal */}
          {memberToRemove && (
            <div className="fixed inset-0 bg-stone-900/50 flex items-center justify-center z-50 p-4">
              <div className="bg-white dark:bg-neutral-900 rounded-2xl p-6 w-full max-w-md border border-stone-200 dark:border-neutral-800">
                <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100 mb-4">
                  {t('team.removeTitle')}
                </h3>

                {error && (
                  <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                    <p className="text-xs text-coral-400">{error}</p>
                  </div>
                )}

                <div className="space-y-4">
                  <div className="text-sm text-stone-400 dark:text-neutral-500">
                    <p>
                      {t('team.removePromptPrefix')}{' '}
                      <strong className="text-stone-900 dark:text-neutral-100">
                        {displayName(memberToRemove)}
                      </strong>{' '}
                      {t('team.removePromptSuffix')}
                    </p>
                    <p className="mt-2 text-coral-400">{t('team.removeWarning')}</p>
                  </div>

                  <div className="flex gap-2 pt-2">
                    <button
                      onClick={() => setMemberToRemove(null)}
                      disabled={removingId === memberToRemove._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-stone-100 dark:bg-neutral-800 hover:bg-stone-200 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200 transition-colors disabled:opacity-50">
                      {t('common.cancel')}
                    </button>
                    <button
                      onClick={confirmRemoveMember}
                      disabled={removingId === memberToRemove._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-coral-500 hover:bg-coral-600 text-white transition-colors disabled:opacity-50">
                      {removingId === memberToRemove._id
                        ? t('team.removing')
                        : t('team.removeAction')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Change Role Confirmation Modal */}
          {roleChangeConfirmation && (
            <div className="fixed inset-0 bg-stone-900/50 flex items-center justify-center z-50 p-4">
              <div className="bg-white dark:bg-neutral-900 rounded-2xl p-6 w-full max-w-md border border-stone-200 dark:border-neutral-800">
                <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100 mb-4">
                  {t('team.changeRoleTitle')}
                </h3>

                {error && (
                  <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                    <p className="text-xs text-coral-400">{error}</p>
                  </div>
                )}

                <div className="space-y-4">
                  <div className="text-sm text-stone-400 dark:text-neutral-500">
                    <p>
                      {t('team.changeRolePrompt')
                        .replace('{name}', displayName(roleChangeConfirmation.member))
                        .replace('{oldRole}', roleChangeConfirmation.oldRole)
                        .replace('{newRole}', roleChangeConfirmation.newRole)}
                    </p>
                    {roleChangeConfirmation.newRole === 'ADMIN' && (
                      <p className="mt-2 text-amber-400">{t('team.changeRoleAdminGrant')}</p>
                    )}
                    {roleChangeConfirmation.oldRole === 'ADMIN' && (
                      <p className="mt-2 text-coral-400">{t('team.changeRoleAdminRemove')}</p>
                    )}
                  </div>

                  <div className="flex gap-2 pt-2">
                    <button
                      onClick={() => setRoleChangeConfirmation(null)}
                      disabled={changingRoleId === roleChangeConfirmation.member._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-stone-700/50 hover:bg-stone-700 text-stone-300 dark:text-neutral-600 transition-colors disabled:opacity-50">
                      {t('common.cancel')}
                    </button>
                    <button
                      onClick={confirmChangeRole}
                      disabled={changingRoleId === roleChangeConfirmation.member._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-primary-500 hover:bg-primary-600 text-white transition-colors disabled:opacity-50">
                      {changingRoleId === roleChangeConfirmation.member._id
                        ? t('team.changing')
                        : t('team.changeRoleAction')}
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

export default TeamMembersPanel;
