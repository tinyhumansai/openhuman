import debug from 'debug';
import { useEffect, useState } from 'react';
import { useLocation, useParams } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { teamApi } from '../../../services/api/teamApi';
import type { TeamMember, TeamRole } from '../../../types/team';
import { sanitizeError } from '../../../utils/sanitize';
import { CenteredLoadingState, ConfirmDialog, ErrorBanner, InlineLoadingStatus } from '../../ui';
import Button from '../../ui/Button';
import { SettingsBadge, SettingsEmptyState, SettingsSection, SettingsSelect } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const log = debug('core-rpc:error');

const ROLES: TeamRole[] = ['ADMIN', 'BILLING_MANAGER', 'MEMBER'];

const TeamMembersPanel = () => {
  const { t } = useT();
  const { teamId } = useParams<{ teamId: string }>();
  const location = useLocation();
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

  const roleBadgeVariant: Record<string, 'primary' | 'warning' | 'neutral'> = {
    ADMIN: 'primary',
    BILLING_MANAGER: 'warning',
    MEMBER: 'neutral',
  };

  return (
    <SettingsPanel title={t('team.members')} description={t('pages.settings.account.teamDesc')}>
      {error && <ErrorBanner message={error} />}

      {/* Refreshing indicator - only when loading and has existing data */}
      {isLoadingMembers && members.length > 0 && (
        <InlineLoadingStatus label={t('team.refreshingMembers')} />
      )}

      {/* Member count */}
      <p className="text-xs text-content-muted px-1">
        {t(members.length === 1 ? 'team.memberCount' : 'team.memberCountPlural').replace(
          '{count}',
          String(members.length)
        )}
      </p>

      {/* Full loading state - only when loading and no existing data */}
      {isLoadingMembers && members.length === 0 ? (
        <CenteredLoadingState label={t('team.loadingMembers')} />
      ) : (
        <SettingsSection>
          {members.length === 0 && !isLoadingMembers ? (
            <SettingsEmptyState label={t('team.noMembers')} />
          ) : (
            <ul>
              {members.map(member => (
                <li
                  key={member._id}
                  className="flex items-center justify-between px-4 py-3 border-b border-line-subtle last:border-b-0">
                  <div className="flex items-center gap-3 min-w-0">
                    {/* Avatar */}
                    <div className="w-8 h-8 rounded-full bg-neutral-700/60 flex items-center justify-center flex-shrink-0">
                      <span className="text-xs font-semibold text-white">
                        {displayName(member).charAt(0).toUpperCase()}
                      </span>
                    </div>
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium text-content truncate">
                          {displayName(member)}
                        </span>
                        {isCurrentUser(member) && (
                          <span className="text-[10px] text-content-muted">{t('team.you')}</span>
                        )}
                      </div>
                      {member.user.username && (
                        <p className="text-xs text-content-muted truncate">
                          @{member.user.username}
                        </p>
                      )}
                    </div>
                  </div>

                  <div className="flex items-center gap-2 flex-shrink-0">
                    {/* Role badge / dropdown */}
                    {isAdmin && !isCurrentUser(member) ? (
                      <SettingsSelect
                        value={member.role.toUpperCase()}
                        onChange={e => handleChangeRole(member, e.target.value as TeamRole)}
                        disabled={changingRoleId === member._id}
                        aria-label={t('team.roleSelectorAria')}
                        inputSize="sm"
                        className="w-36">
                        {ROLES.map(r => (
                          <option key={r} value={r}>
                            {r}
                          </option>
                        ))}
                      </SettingsSelect>
                    ) : (
                      <SettingsBadge
                        variant={roleBadgeVariant[member.role.toUpperCase()] ?? 'neutral'}>
                        {member.role.toUpperCase()}
                      </SettingsBadge>
                    )}

                    {/* Remove button (admin only, not self) */}
                    {isAdmin && !isCurrentUser(member) && (
                      <Button
                        type="button"
                        variant="tertiary"
                        size="xs"
                        onClick={() => handleRemoveMember(member)}
                        disabled={removingId === member._id}
                        aria-label={t('team.removeAria').replace('{name}', displayName(member))}
                        className="text-content-muted hover:text-coral-400 hover:bg-coral-500/10">
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
                      </Button>
                    )}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </SettingsSection>
      )}

      {/* Remove Member Confirmation Modal */}
      {memberToRemove && (
        <ConfirmDialog
          title={t('team.removeTitle')}
          body={
            <div className="space-y-4">
              {error && <ErrorBanner message={error} />}
              <div className="text-sm text-content-muted">
                <p>
                  {t('team.removePromptPrefix')}{' '}
                  <strong className="text-content">{displayName(memberToRemove)}</strong>{' '}
                  {t('team.removePromptSuffix')}
                </p>
                <p className="mt-2 text-coral-400">{t('team.removeWarning')}</p>
              </div>
            </div>
          }
          confirmLabel={t('team.removeAction')}
          cancelLabel={t('common.cancel')}
          busy={removingId === memberToRemove._id}
          busyLabel={t('team.removing')}
          destructive
          onConfirm={() => void confirmRemoveMember()}
          onCancel={() => setMemberToRemove(null)}
        />
      )}

      {/* Change Role Confirmation Modal */}
      {roleChangeConfirmation && (
        <ConfirmDialog
          title={t('team.changeRoleTitle')}
          body={
            <div className="space-y-4">
              {error && <ErrorBanner message={error} />}
              <div className="text-sm text-content-muted">
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
            </div>
          }
          confirmLabel={t('team.changeRoleAction')}
          cancelLabel={t('common.cancel')}
          busy={changingRoleId === roleChangeConfirmation.member._id}
          busyLabel={t('team.changing')}
          destructive={false}
          onConfirm={() => void confirmChangeRole()}
          onCancel={() => setRoleChangeConfirmation(null)}
        />
      )}
    </SettingsPanel>
  );
};

export default TeamMembersPanel;
