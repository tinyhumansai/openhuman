import { useCallback, useEffect, useState } from 'react';

import { useCoreState } from '../../../providers/CoreStateProvider';
import { teamApi } from '../../../services/api/teamApi';
import type { TeamWithRole } from '../../../types/team';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const TeamPanel = () => {
  const { navigateBack, navigateToTeamManagement } = useSettingsNavigation();
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

  // Confirmation modal state for leaving team
  const [teamToLeave, setTeamToLeave] = useState<TeamWithRole | null>(null);

  const activeTeamId = user?.activeTeamId;

  const refreshTeamsWithLoading = useCallback(async () => {
    setIsLoading(true);
    try {
      await refreshTeams();
    } finally {
      setIsLoading(false);
    }
  }, [refreshTeams]);

  useEffect(() => {
    void refreshTeamsWithLoading();
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
      await Promise.all([refresh(), refreshTeamsWithLoading()]);
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
      await Promise.all([refresh(), refreshTeamsWithLoading()]);
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

  const handleLeaveTeam = (teamEntry: TeamWithRole) => {
    // Show confirmation modal for leaving teams
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
          : 'Failed to leave team'
      );
    } finally {
      setIsLeaving(null);
    }
  };

  const roleBadge = (role: string, teamCreatedBy?: string) => {
    // Normalize role to uppercase for consistent comparison
    const normalizedRole = role.toUpperCase();

    // Show "Owner" if this is the team creator and admin
    const isOwner = normalizedRole === 'ADMIN' && teamCreatedBy === user?._id;

    const roleLabel = isOwner
      ? 'Owner'
      : normalizedRole === 'ADMIN'
        ? 'Admin'
        : normalizedRole === 'BILLING_MANAGER'
          ? 'Billing Manager'
          : 'Member';

    const colors: Record<string, string> = {
      ADMIN: 'bg-primary-500/20 text-primary-400 border-primary-500/30',
      BILLING_MANAGER: 'bg-amber-500/20 text-amber-400 border-amber-500/30',
      MEMBER: 'bg-stone-500/20 text-stone-400 border-stone-500/30',
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
    const normalizedRole = role.toUpperCase();
    const canLeave = !team.isPersonal && normalizedRole !== 'ADMIN';
    const canManage = normalizedRole === 'ADMIN' && !team.isPersonal;

    return (
      <div
        className={`flex items-center justify-between p-3 rounded-xl border transition-all ${
          isActive
            ? 'border-primary-200 bg-primary-50'
            : 'border-stone-200 bg-white hover:bg-stone-50'
        }`}>
        <div className="flex items-center gap-3 min-w-0 flex-1">
          {/* Team avatar */}
          <div className="w-9 h-9 rounded-lg bg-stone-100 flex items-center justify-center flex-shrink-0">
            <span className="text-sm font-semibold text-stone-600">
              {team.name.charAt(0).toUpperCase()}
            </span>
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-sm font-medium text-stone-900 truncate">{team.name}</span>
              {roleBadge(role, team.createdBy)}
              {planBadge(team.subscription.plan)}
              {isActive && (
                <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-sage-500/20 text-sage-400 border border-sage-500/30">
                  Active
                </span>
              )}
            </div>
            {team.isPersonal && <p className="text-xs text-stone-400 mt-0.5">Personal team</p>}
          </div>
        </div>

        <div className="flex items-center gap-2 flex-shrink-0">
          {canManage && (
            <button
              onClick={() => navigateToTeamManagement(team._id)}
              className="px-2.5 py-1 text-xs font-medium rounded-lg bg-primary-50 hover:bg-primary-100 text-primary-600 transition-colors">
              Manage Team
            </button>
          )}
          {!isActive && (
            <button
              onClick={() => handleSwitchTeam(team._id)}
              disabled={isSwitching === team._id}
              className="px-2.5 py-1 text-xs font-medium rounded-lg bg-stone-100 hover:bg-stone-200 text-stone-600 transition-colors disabled:opacity-50">
              {isSwitching === team._id ? 'Switching...' : 'Switch'}
            </button>
          )}
          {canLeave && (
            <button
              onClick={() => handleLeaveTeam(entry)}
              disabled={isLeaving === team._id}
              className="px-2.5 py-1 text-xs font-medium rounded-lg text-amber-700 hover:bg-amber-50 transition-colors disabled:opacity-50">
              {isLeaving === team._id ? 'Leaving...' : 'Leave'}
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
        <div className="max-w-md mx-auto p-4 space-y-4">
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

          {/* Teams List - Primary Content */}
          {teams.length > 0 && (
            <div className="space-y-3">
              <h3 className="text-xs font-medium text-stone-500 uppercase tracking-wider px-1">
                Your Teams ({teams.length})
              </h3>
              <div className="space-y-2">
                {teams.map(entry => (
                  <TeamRow key={entry.team._id} entry={entry} />
                ))}
              </div>
            </div>
          )}

          {/* Team Actions - Secondary Content */}
          <div className="space-y-4 border-t border-stone-200 pt-4">
            {/* Create team */}
            <div className="space-y-2">
              <h3 className="text-xs font-medium text-stone-500 uppercase tracking-wider px-1">
                Create New Team
              </h3>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={newTeamName}
                  onChange={e => setNewTeamName(e.target.value)}
                  onKeyDown={e => e.key === 'Enter' && handleCreateTeam()}
                  placeholder="Team name"
                  className="flex-1 px-3 py-2 text-sm bg-white border border-stone-200 rounded-xl text-stone-900 placeholder-stone-400 focus:outline-none focus:border-primary-500/50"
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
                Join Existing Team
              </h3>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={joinCode}
                  onChange={e => setJoinCode(e.target.value)}
                  onKeyDown={e => e.key === 'Enter' && handleJoinTeam()}
                  placeholder="Invite code"
                  className="flex-1 px-3 py-2 text-sm bg-white border border-stone-200 rounded-xl text-stone-900 placeholder-stone-400 focus:outline-none focus:border-primary-500/50 font-mono"
                />
                <button
                  onClick={handleJoinTeam}
                  disabled={isJoining || !joinCode.trim()}
                  className="px-4 py-2 text-xs font-medium rounded-xl bg-stone-100 hover:bg-stone-200 text-stone-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
                  {isJoining ? 'Joining...' : 'Join'}
                </button>
              </div>
            </div>
          </div>

          {/* Leave Team Confirmation Modal */}
          {teamToLeave && (
            <div className="fixed inset-0 bg-stone-900/50 flex items-center justify-center z-50 p-4">
              <div className="bg-white rounded-2xl p-6 w-full max-w-md border border-stone-200">
                <h3 className="text-lg font-semibold text-stone-900 mb-4">Leave Team</h3>

                {error && (
                  <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                    <p className="text-xs text-coral-400">{error}</p>
                  </div>
                )}

                <div className="space-y-4">
                  <div className="text-sm text-stone-500">
                    <p>
                      Are you sure you want to leave{' '}
                      <strong className="text-stone-900">{teamToLeave.team.name}</strong>?
                    </p>
                    <p className="mt-2 text-amber-400">
                      You will lose access to the team and all team resources. You'll need a new
                      invite to rejoin.
                    </p>
                  </div>

                  <div className="flex gap-2 pt-2">
                    <button
                      onClick={() => setTeamToLeave(null)}
                      disabled={isLeaving === teamToLeave.team._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-stone-100 hover:bg-stone-200 text-stone-600 transition-colors disabled:opacity-50">
                      Cancel
                    </button>
                    <button
                      onClick={confirmLeaveTeam}
                      disabled={isLeaving === teamToLeave.team._id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-amber-500 hover:bg-amber-600 text-white transition-colors disabled:opacity-50">
                      {isLeaving === teamToLeave.team._id ? 'Leaving...' : 'Leave Team'}
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
