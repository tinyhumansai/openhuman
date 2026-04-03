import { useEffect, useState } from 'react';
import { useLocation, useParams } from 'react-router-dom';

import { teamApi } from '../../../services/api/teamApi';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { fetchMembers } from '../../../store/teamSlice';
import type { TeamMember, TeamRole } from '../../../types/team';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const ROLES: TeamRole[] = ['ADMIN', 'BILLING_MANAGER', 'MEMBER'];

const TeamMembersPanel = () => {
  const { teamId } = useParams<{ teamId: string }>();
  const location = useLocation();
  const { navigateBack } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const user = useAppSelector(state => state.user.user);
  const { teams, members, isLoadingMembers } = useAppSelector(state => state.team);

  // Check if we're in team management context (has teamId in URL)
  const isInManagementContext = location.pathname.includes('/team/manage/');
  const currentTeamId = isInManagementContext ? teamId : user?.activeTeamId;
  const currentTeam = teams.find(t => t.team._id === currentTeamId);
  const isAdmin = currentTeam?.role.toUpperCase() === 'ADMIN';

  const [removingId, setRemovingId] = useState<string | null>(null);
  const [changingRoleId, setChangingRoleId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Confirmation modals state
  const [memberToRemove, setMemberToRemove] = useState<TeamMember | null>(null);
  const [roleChangeConfirmation, setRoleChangeConfirmation] = useState<{
    member: TeamMember;
    newRole: TeamRole;
    oldRole: TeamRole;
  } | null>(null);

  useEffect(() => {
    if (currentTeamId) dispatch(fetchMembers(currentTeamId));
  }, [currentTeamId, dispatch]);

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
      dispatch(fetchMembers(currentTeamId));
      setRoleChangeConfirmation(null);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Failed to change role'
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
      dispatch(fetchMembers(currentTeamId));
      setMemberToRemove(null);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Failed to remove member'
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
    MEMBER: 'bg-stone-500/20 text-stone-400 border-stone-500/30',
  };

  return (
    <div className="overflow-hidden flex flex-col h-full">
      <SettingsHeader title="Members" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-md mx-auto p-4 space-y-3">
          {error && (
            <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3">
              <p className="text-xs text-coral-400">{error}</p>
            </div>
          )}

          {/* Refreshing indicator - only when loading and has existing data */}
          {isLoadingMembers && members.length > 0 && (
            <div className="flex items-center gap-2 px-1 py-2 text-xs text-amber-400">
              <svg className="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                />
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                />
              </svg>
              Refreshing members...
            </div>
          )}

          {/* Member count */}
          <p className="text-xs text-stone-500 px-1">
            {members.length} member{members.length !== 1 ? 's' : ''}
          </p>

          {/* Full loading state - only when loading and no existing data */}
          {isLoadingMembers && members.length === 0 ? (
            <div className="flex items-center justify-center py-8">
              <svg className="w-5 h-5 text-stone-500 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                />
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                />
              </svg>
              <span className="ml-3 text-sm text-stone-500">Loading members...</span>
            </div>
          ) : (
            <div className="space-y-2">
              {members.map(member => (
                <div
                  key={member._id}
                  className="flex items-center justify-between p-3 rounded-xl border border-stone-200 bg-white">
                  <div className="flex items-center gap-3 min-w-0">
                    {/* Avatar */}
                    <div className="w-8 h-8 rounded-full bg-stone-700/60 flex items-center justify-center flex-shrink-0">
                      <span className="text-xs font-semibold text-stone-600">
                        {displayName(member).charAt(0).toUpperCase()}
                      </span>
                    </div>
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium text-stone-900 truncate">
                          {displayName(member)}
                        </span>
                        {isCurrentUser(member) && (
                          <span className="text-[10px] text-stone-500">(You)</span>
                        )}
                      </div>
                      {member.user.username && (
                        <p className="text-xs text-stone-500 truncate">@{member.user.username}</p>
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
                        className="px-2 py-1 text-[10px] font-medium rounded-full border bg-white text-stone-700 border-stone-300 focus:outline-none focus:border-primary-500/50 disabled:opacity-50">
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
                        className="p-1 rounded-lg text-stone-500 hover:text-coral-400 hover:bg-coral-500/10 transition-colors disabled:opacity-50"
                        aria-label={`Remove ${displayName(member)}`}>
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
                  <p className="text-sm text-stone-500">No members found</p>
                </div>
              )}
            </div>
          )}

          {/* Remove Member Confirmation Modal */}
          {memberToRemove && (
            <div className="fixed inset-0 bg-stone-900/50 flex items-center justify-center z-50 p-4">
              <div className="bg-white rounded-2xl p-6 w-full max-w-md border border-stone-200">
                <h3 className="text-lg font-semibold text-stone-900 mb-4">Remove Team Member</h3>

                {error && (
                  <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                    <p className="text-xs text-coral-400">{error}</p>
                  </div>
                )}

                <div className="space-y-4">
                  <div className="text-sm text-stone-400">
                    <p>
                      Are you sure you want to remove{' '}
                      <strong className="text-stone-900">{displayName(memberToRemove)}</strong> from
                      the team?
                    </p>
                    <p className="mt-2 text-coral-400">
                      They will lose access to the team and all team resources.
                    </p>
                  </div>

                  <div className="flex gap-2 pt-2">
                    <button
                      onClick={() => setMemberToRemove(null)}
                      disabled={removingId === memberToRemove._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-stone-100 hover:bg-stone-200 text-stone-700 transition-colors disabled:opacity-50">
                      Cancel
                    </button>
                    <button
                      onClick={confirmRemoveMember}
                      disabled={removingId === memberToRemove._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-coral-500 hover:bg-coral-600 text-white transition-colors disabled:opacity-50">
                      {removingId === memberToRemove._id ? 'Removing...' : 'Remove Member'}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Change Role Confirmation Modal */}
          {roleChangeConfirmation && (
            <div className="fixed inset-0 bg-stone-900/50 flex items-center justify-center z-50 p-4">
              <div className="bg-white rounded-2xl p-6 w-full max-w-md border border-stone-200">
                <h3 className="text-lg font-semibold text-stone-900 mb-4">Change Member Role</h3>

                {error && (
                  <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                    <p className="text-xs text-coral-400">{error}</p>
                  </div>
                )}

                <div className="space-y-4">
                  <div className="text-sm text-stone-400">
                    <p>
                      Change{' '}
                      <strong className="text-white">
                        {displayName(roleChangeConfirmation.member)}
                      </strong>
                      's role from{' '}
                      <span className="text-amber-400 font-medium">
                        {roleChangeConfirmation.oldRole}
                      </span>{' '}
                      to{' '}
                      <span className="text-primary-400 font-medium">
                        {roleChangeConfirmation.newRole}
                      </span>
                      ?
                    </p>
                    {roleChangeConfirmation.newRole === 'ADMIN' && (
                      <p className="mt-2 text-amber-400">
                        This will grant them full admin permissions including the ability to manage
                        team members.
                      </p>
                    )}
                    {roleChangeConfirmation.oldRole === 'ADMIN' && (
                      <p className="mt-2 text-coral-400">
                        This will remove their admin permissions and they will no longer be able to
                        manage the team.
                      </p>
                    )}
                  </div>

                  <div className="flex gap-2 pt-2">
                    <button
                      onClick={() => setRoleChangeConfirmation(null)}
                      disabled={changingRoleId === roleChangeConfirmation.member._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-stone-700/50 hover:bg-stone-700 text-stone-300 transition-colors disabled:opacity-50">
                      Cancel
                    </button>
                    <button
                      onClick={confirmChangeRole}
                      disabled={changingRoleId === roleChangeConfirmation.member._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-primary-500 hover:bg-primary-600 text-white transition-colors disabled:opacity-50">
                      {changingRoleId === roleChangeConfirmation.member._id
                        ? 'Changing...'
                        : 'Change Role'}
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
