import { useEffect, useState } from 'react';

import { teamApi } from '../../../services/api/teamApi';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { fetchTeams } from '../../../store/teamSlice';
import { fetchCurrentUser } from '../../../store/userSlice';
import type { TeamWithRole } from '../../../types/team';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const TeamPanel = () => {
  const { navigateBack, navigateToSettings } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const user = useAppSelector(state => state.user.user);
  const { teams, isLoading } = useAppSelector(state => state.team);

  const [newTeamName, setNewTeamName] = useState('');
  const [joinCode, setJoinCode] = useState('');
  const [isCreating, setIsCreating] = useState(false);
  const [isJoining, setIsJoining] = useState(false);
  const [isSwitching, setIsSwitching] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const activeTeamId = user?.activeTeamId;
  const activeTeamEntry = teams.find(t => t.team._id === activeTeamId);
  const isAdmin = activeTeamEntry?.role === 'ADMIN';

  useEffect(() => {
    dispatch(fetchTeams());
  }, [dispatch]);

  const handleCreateTeam = async () => {
    const name = newTeamName.trim();
    if (!name) return;
    setIsCreating(true);
    setError(null);
    try {
      await teamApi.createTeam(name);
      setNewTeamName('');
      dispatch(fetchTeams());
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Failed to create team'
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
      await dispatch(fetchCurrentUser());
      dispatch(fetchTeams());
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Invalid or expired invite code'
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
      await dispatch(fetchCurrentUser());
      dispatch(fetchTeams());
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Failed to switch team'
      );
    } finally {
      setIsSwitching(null);
    }
  };

  const handleLeaveTeam = async (teamId: string) => {
    setError(null);
    try {
      await teamApi.leaveTeam(teamId);
      await dispatch(fetchCurrentUser());
      dispatch(fetchTeams());
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Failed to leave team'
      );
    }
  };

  const roleBadge = (role: string) => {
    const colors: Record<string, string> = {
      ADMIN: 'bg-primary-500/20 text-primary-400 border-primary-500/30',
      BILLING_MANAGER: 'bg-amber-500/20 text-amber-400 border-amber-500/30',
      MEMBER: 'bg-stone-500/20 text-stone-400 border-stone-500/30',
    };
    return (
      <span
        className={`px-1.5 py-0.5 text-[10px] font-medium rounded-full border ${colors[role] ?? colors.MEMBER}`}>
        {role}
      </span>
    );
  };

  const planBadge = (plan: string) => {
    const colors: Record<string, string> = {
      PRO: 'bg-lavender-500/20 text-lavender-400 border-lavender-500/30',
      BASIC: 'bg-primary-500/20 text-primary-400 border-primary-500/30',
      FREE: 'bg-stone-500/20 text-stone-400 border-stone-500/30',
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
    const canLeave = !team.isPersonal && role !== 'ADMIN';

    return (
      <div
        className={`flex items-center justify-between p-3 rounded-xl border transition-all ${
          isActive
            ? 'border-primary-500/40 bg-primary-500/5'
            : 'border-stone-700/50 bg-stone-800/40 hover:bg-stone-800/60'
        }`}>
        <div className="flex items-center gap-3 min-w-0 flex-1">
          {/* Team avatar */}
          <div className="w-9 h-9 rounded-lg bg-stone-700/60 flex items-center justify-center flex-shrink-0">
            <span className="text-sm font-semibold text-stone-300">
              {team.name.charAt(0).toUpperCase()}
            </span>
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-sm font-medium text-white truncate">{team.name}</span>
              {roleBadge(role)}
              {planBadge(team.subscription.plan)}
              {isActive && (
                <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-sage-500/20 text-sage-400 border border-sage-500/30">
                  Active
                </span>
              )}
            </div>
            {team.isPersonal && <p className="text-xs text-stone-500 mt-0.5">Personal team</p>}
          </div>
        </div>

        <div className="flex items-center gap-2 flex-shrink-0">
          {!isActive && (
            <button
              onClick={() => handleSwitchTeam(team._id)}
              disabled={isSwitching === team._id}
              className="px-2.5 py-1 text-xs font-medium rounded-lg bg-stone-700/50 hover:bg-stone-700 text-stone-300 transition-colors disabled:opacity-50">
              {isSwitching === team._id ? 'Switching...' : 'Switch'}
            </button>
          )}
          {canLeave && (
            <button
              onClick={() => handleLeaveTeam(team._id)}
              className="px-2.5 py-1 text-xs font-medium rounded-lg text-amber-400 hover:bg-amber-500/10 transition-colors">
              Leave
            </button>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="overflow-hidden flex flex-col h-full">
      <SettingsHeader title="Team" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto">
        <div className="p-4 space-y-4">
          {/* Error banner */}
          {error && (
            <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3">
              <p className="text-xs text-coral-400">{error}</p>
            </div>
          )}

          {/* Loading */}
          {isLoading && teams.length === 0 && (
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
            </div>
          )}

          {/* Team list */}
          {teams.length > 0 && (
            <div className="space-y-2">
              <h3 className="text-xs font-medium text-stone-500 uppercase tracking-wider px-1">
                Your Teams
              </h3>
              {teams.map(entry => (
                <TeamRow key={entry.team._id} entry={entry} />
              ))}
            </div>
          )}

          {/* Admin actions for active team */}
          {isAdmin && activeTeamEntry && !activeTeamEntry.team.isPersonal && (
            <div className="space-y-1">
              <h3 className="text-xs font-medium text-stone-500 uppercase tracking-wider px-1">
                Manage Team
              </h3>
              <button
                onClick={() => navigateToSettings('team-members')}
                className="w-full flex items-center justify-between p-3 bg-black/50 border-b border-stone-700 hover:bg-stone-800/30 transition-all text-left first:rounded-t-xl">
                <div className="flex items-center gap-3">
                  <svg
                    className="w-5 h-5 opacity-60 text-white"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 4.354a4 4 0 110 5.292M15 21H3v-1a6 6 0 0112 0v1zm0 0h6v-1a6 6 0 00-9-5.197m13.5-9a2.5 2.5 0 11-5 0 2.5 2.5 0 015 0z"
                    />
                  </svg>
                  <div>
                    <div className="font-medium text-sm text-white">Members</div>
                    <p className="text-xs opacity-70">Manage team members and roles</p>
                  </div>
                </div>
                <svg
                  className="w-4 h-4 text-stone-500"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M9 5l7 7-7 7"
                  />
                </svg>
              </button>
              <button
                onClick={() => navigateToSettings('team-invites')}
                className="w-full flex items-center justify-between p-3 bg-black/50 hover:bg-stone-800/30 transition-all text-left last:rounded-b-xl">
                <div className="flex items-center gap-3">
                  <svg
                    className="w-5 h-5 opacity-60 text-white"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
                    />
                  </svg>
                  <div>
                    <div className="font-medium text-sm text-white">Invites</div>
                    <p className="text-xs opacity-70">Generate and manage invite codes</p>
                  </div>
                </div>
                <svg
                  className="w-4 h-4 text-stone-500"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M9 5l7 7-7 7"
                  />
                </svg>
              </button>
            </div>
          )}

          {/* Create team */}
          <div className="space-y-2">
            <h3 className="text-xs font-medium text-stone-500 uppercase tracking-wider px-1">
              Create a Team
            </h3>
            <div className="flex gap-2">
              <input
                type="text"
                value={newTeamName}
                onChange={e => setNewTeamName(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && handleCreateTeam()}
                placeholder="Team name"
                className="flex-1 px-3 py-2 text-sm bg-stone-800/60 border border-stone-700/50 rounded-xl text-white placeholder-stone-500 focus:outline-none focus:border-primary-500/50"
              />
              <button
                onClick={handleCreateTeam}
                disabled={isCreating || !newTeamName.trim()}
                className="px-4 py-2 text-xs font-medium rounded-xl bg-primary-500 hover:bg-primary-600 text-white transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
                {isCreating ? 'Creating...' : 'Create'}
              </button>
            </div>
          </div>

          {/* Join team */}
          <div className="space-y-2">
            <h3 className="text-xs font-medium text-stone-500 uppercase tracking-wider px-1">
              Join a Team
            </h3>
            <div className="flex gap-2">
              <input
                type="text"
                value={joinCode}
                onChange={e => setJoinCode(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && handleJoinTeam()}
                placeholder="Invite code"
                className="flex-1 px-3 py-2 text-sm bg-stone-800/60 border border-stone-700/50 rounded-xl text-white placeholder-stone-500 focus:outline-none focus:border-primary-500/50 font-mono"
              />
              <button
                onClick={handleJoinTeam}
                disabled={isJoining || !joinCode.trim()}
                className="px-4 py-2 text-xs font-medium rounded-xl bg-stone-700/50 hover:bg-stone-700 text-stone-300 transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
                {isJoining ? 'Joining...' : 'Join'}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default TeamPanel;
