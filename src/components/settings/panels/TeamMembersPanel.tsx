import { useEffect, useState } from 'react';

import { teamApi } from '../../../services/api/teamApi';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { fetchMembers } from '../../../store/teamSlice';
import type { TeamMember, TeamRole } from '../../../types/team';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const ROLES: TeamRole[] = ['ADMIN', 'BILLING_MANAGER', 'MEMBER'];

const TeamMembersPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const user = useAppSelector(state => state.user.user);
  const { teams, members } = useAppSelector(state => state.team);

  const activeTeamId = user?.activeTeamId;
  const activeTeam = teams.find(t => t.team._id === activeTeamId);
  const isAdmin = activeTeam?.role === 'ADMIN';

  const [removingId, setRemovingId] = useState<string | null>(null);
  const [changingRoleId, setChangingRoleId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (activeTeamId) dispatch(fetchMembers(activeTeamId));
  }, [activeTeamId, dispatch]);

  const handleChangeRole = async (member: TeamMember, newRole: TeamRole) => {
    if (!activeTeamId || member.role === newRole) return;
    setChangingRoleId(member._id);
    setError(null);
    try {
      await teamApi.changeMemberRole(activeTeamId, member.user._id, newRole);
      dispatch(fetchMembers(activeTeamId));
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

  const handleRemoveMember = async (member: TeamMember) => {
    if (!activeTeamId) return;
    setRemovingId(member._id);
    setError(null);
    try {
      await teamApi.removeMember(activeTeamId, member.user._id);
      dispatch(fetchMembers(activeTeamId));
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
        <div className="p-4 space-y-3">
          {error && (
            <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3">
              <p className="text-xs text-coral-400">{error}</p>
            </div>
          )}

          <p className="text-xs text-stone-500 px-1">
            {members.length} member{members.length !== 1 ? 's' : ''}
          </p>

          <div className="space-y-2">
            {members.map(member => (
              <div
                key={member._id}
                className="flex items-center justify-between p-3 rounded-xl border border-stone-700/50 bg-stone-800/40">
                <div className="flex items-center gap-3 min-w-0">
                  {/* Avatar */}
                  <div className="w-8 h-8 rounded-full bg-stone-700/60 flex items-center justify-center flex-shrink-0">
                    <span className="text-xs font-semibold text-stone-300">
                      {displayName(member).charAt(0).toUpperCase()}
                    </span>
                  </div>
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-white truncate">
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
                      value={member.role}
                      onChange={e => handleChangeRole(member, e.target.value as TeamRole)}
                      disabled={changingRoleId === member._id}
                      className="px-2 py-1 text-[10px] font-medium rounded-full border bg-stone-800 text-stone-300 border-stone-600 focus:outline-none focus:border-primary-500/50 disabled:opacity-50">
                      {ROLES.map(r => (
                        <option key={r} value={r}>
                          {r}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <span
                      className={`px-1.5 py-0.5 text-[10px] font-medium rounded-full border ${roleBadgeColor[member.role] ?? roleBadgeColor.MEMBER}`}>
                      {member.role}
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
          </div>

          {members.length === 0 && (
            <div className="text-center py-8">
              <p className="text-sm text-stone-500">No members found</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default TeamMembersPanel;
