import { useEffect, useRef, useState } from 'react';
import { useParams } from 'react-router-dom';

import { useCoreState } from '../../../providers/CoreStateProvider';
import { teamApi } from '../../../services/api/teamApi';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const TeamManagementPanel = () => {
  const { teamId } = useParams<{ teamId: string }>();
  const { navigateBack, navigateToSettings } = useSettingsNavigation();
  const { teams, refreshTeams } = useCoreState();
  const initialFetchAttemptedRef = useRef(false);

  const teamEntry = teams.find(t => t.team._id === teamId);
  const isAdmin = teamEntry?.role.toUpperCase() === 'ADMIN';

  // State for edit/delete operations
  const [isEditModalOpen, setIsEditModalOpen] = useState(false);
  const [isDeleteModalOpen, setIsDeleteModalOpen] = useState(false);
  const [editTeamName, setEditTeamName] = useState('');
  const [isUpdating, setIsUpdating] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (teams.length > 0) {
      initialFetchAttemptedRef.current = true;
      return;
    }

    if (!initialFetchAttemptedRef.current) {
      initialFetchAttemptedRef.current = true;
      void refreshTeams();
    }
  }, [refreshTeams, teams.length]);

  // Redirect if user doesn't have admin access to this team
  useEffect(() => {
    if (teamEntry && !isAdmin) {
      navigateBack();
    }
  }, [teamEntry, isAdmin, navigateBack]);

  // Handlers for edit/delete operations
  const handleEditTeam = () => {
    setEditTeamName(teamEntry?.team.name || '');
    setError(null);
    setIsEditModalOpen(true);
  };

  const handleUpdateTeam = async () => {
    if (!teamId || !editTeamName.trim()) return;
    setIsUpdating(true);
    setError(null);
    try {
      await teamApi.updateTeam(teamId, { name: editTeamName.trim() });
      await refreshTeams();
      setIsEditModalOpen(false);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Failed to update team'
      );
    } finally {
      setIsUpdating(false);
    }
  };

  const handleDeleteTeam = async () => {
    if (!teamId) return;
    setIsDeleting(true);
    setError(null);
    try {
      await teamApi.deleteTeam(teamId);
      await refreshTeams();
      navigateBack(); // Navigate back after deletion
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Failed to delete team'
      );
      setIsDeleting(false);
    }
  };

  if (!teamEntry) {
    return (
      <div className="">
        <SettingsHeader title="Team Management" showBackButton={true} onBack={navigateBack} />
        <div className="flex-1 flex items-center justify-center">
          <p className="text-sm text-stone-500">Team not found</p>
        </div>
      </div>
    );
  }

  if (!isAdmin) {
    return (
      <div className="">
        <SettingsHeader title="Team Management" showBackButton={true} onBack={navigateBack} />
        <div className="flex-1 flex items-center justify-center">
          <p className="text-sm text-stone-500">Access denied</p>
        </div>
      </div>
    );
  }

  const { team } = teamEntry;

  return (
    <div className="">
      <SettingsHeader title={`Manage ${team.name}`} showBackButton={true} onBack={navigateBack} />

      <div>
        <div className="max-w-md mx-auto p-4 space-y-4">
          {/* Team Info */}
          <div className="rounded-xl border border-stone-200 bg-stone-50 p-4">
            <div className="flex items-center gap-3 mb-3">
              <div className="w-10 h-10 rounded-lg bg-stone-200 flex items-center justify-center">
                <span className="text-sm font-semibold text-stone-700">
                  {team.name.charAt(0).toUpperCase()}
                </span>
              </div>
              <div>
                <h3 className="text-sm font-semibold text-stone-900">{team.name}</h3>
                <p className="text-xs text-stone-500">
                  {team.subscription.plan} Plan • Created{' '}
                  {new Date(team.createdAt).toLocaleDateString()}
                </p>
              </div>
            </div>
          </div>

          {/* Management Options */}
          <div className="space-y-1">
            <h3 className="text-xs font-medium text-stone-500 uppercase tracking-wider px-1 mb-3">
              Team Management
            </h3>

            {/* Members */}
            <button
              onClick={() => navigateToSettings(`team/manage/${teamId}/members`)}
              className="w-full flex items-center justify-between p-3 rounded-xl border border-stone-200 bg-stone-50 hover:bg-stone-100 transition-all text-left">
              <div className="flex items-center gap-3">
                <svg
                  className="w-5 h-5 text-primary-500"
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
                  <div className="font-medium text-sm text-stone-900">Members</div>
                  <p className="text-xs text-stone-500">Manage team members and roles</p>
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

            {/* Invites */}
            <button
              onClick={() => navigateToSettings(`team/manage/${teamId}/invites`)}
              className="w-full flex items-center justify-between p-3 rounded-xl border border-stone-200 bg-stone-50 hover:bg-stone-100 transition-all text-left">
              <div className="flex items-center gap-3">
                <svg
                  className="w-5 h-5 text-primary-500"
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
                  <div className="font-medium text-sm text-stone-900">Invites</div>
                  <p className="text-xs text-stone-500">Generate and manage invite codes</p>
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

            {/* Edit Team Settings */}
            <button
              onClick={handleEditTeam}
              className="w-full flex items-center justify-between p-3 rounded-xl border border-stone-200 bg-stone-50 hover:bg-stone-100 transition-all text-left">
              <div className="flex items-center gap-3">
                <svg
                  className="w-5 h-5 text-primary-500"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
                  />
                </svg>
                <div>
                  <div className="font-medium text-sm text-stone-900">Team Settings</div>
                  <p className="text-xs text-stone-500">Edit team name and settings</p>
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

            {/* Delete Team */}
            {!teamEntry?.team.isPersonal && (
              <button
                onClick={() => setIsDeleteModalOpen(true)}
                className="w-full flex items-center justify-between p-3 rounded-xl border border-coral-500/30 bg-coral-500/5 hover:bg-coral-500/10 transition-all text-left">
                <div className="flex items-center gap-3">
                  <svg
                    className="w-5 h-5 text-coral-400"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                    />
                  </svg>
                  <div>
                    <div className="font-medium text-sm text-coral-400">Delete Team</div>
                    <p className="text-xs text-stone-500">Permanently delete this team</p>
                  </div>
                </div>
                <svg
                  className="w-4 h-4 text-coral-400"
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
            )}
          </div>

          {/* Edit Team Modal */}
          {isEditModalOpen && (
            <div className="fixed inset-0 bg-stone-900/40 flex items-center justify-center z-50 p-4">
              <div className="bg-white rounded-2xl p-6 w-full max-w-md border border-stone-200">
                <h3 className="text-sm font-semibold text-stone-900 mb-4">Edit Team Settings</h3>

                {error && (
                  <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                    <p className="text-xs text-coral-600">{error}</p>
                  </div>
                )}

                <div className="space-y-4">
                  <div>
                    <label className="block text-sm font-medium text-stone-700 mb-2">
                      Team Name
                    </label>
                    <input
                      type="text"
                      value={editTeamName}
                      onChange={e => setEditTeamName(e.target.value)}
                      onKeyDown={e => e.key === 'Enter' && handleUpdateTeam()}
                      className="w-full px-3 py-2 text-sm bg-stone-50 border border-stone-200 rounded-xl text-stone-900 placeholder-stone-400 focus:outline-none focus:border-primary-500/50"
                      placeholder="Enter team name"
                    />
                  </div>

                  <div className="flex gap-2 pt-2">
                    <button
                      onClick={() => setIsEditModalOpen(false)}
                      disabled={isUpdating}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-stone-100 hover:bg-stone-200 text-stone-700 transition-colors disabled:opacity-50">
                      Cancel
                    </button>
                    <button
                      onClick={handleUpdateTeam}
                      disabled={isUpdating || !editTeamName.trim()}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-primary-500 hover:bg-primary-600 text-white transition-colors disabled:opacity-50">
                      {isUpdating ? 'Saving...' : 'Save Changes'}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Delete Team Modal */}
          {isDeleteModalOpen && (
            <div className="fixed inset-0 bg-stone-900/40 flex items-center justify-center z-50 p-4">
              <div className="bg-white rounded-2xl p-6 w-full max-w-md border border-stone-200">
                <h3 className="text-sm font-semibold text-stone-900 mb-4">Delete Team</h3>

                {error && (
                  <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                    <p className="text-xs text-coral-600">{error}</p>
                  </div>
                )}

                <div className="space-y-4">
                  <div className="text-sm text-stone-400">
                    <p>
                      Are you sure you want to delete{' '}
                      <strong className="text-stone-900">{teamEntry?.team.name}</strong>?
                    </p>
                    <p className="mt-2 text-coral-400">
                      This action cannot be undone. All team data will be permanently removed.
                    </p>
                  </div>

                  <div className="flex gap-2 pt-2">
                    <button
                      onClick={() => setIsDeleteModalOpen(false)}
                      disabled={isDeleting}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-stone-100 hover:bg-stone-200 text-stone-700 transition-colors disabled:opacity-50">
                      Cancel
                    </button>
                    <button
                      onClick={handleDeleteTeam}
                      disabled={isDeleting}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-coral-500 hover:bg-coral-600 text-white transition-colors disabled:opacity-50">
                      {isDeleting ? 'Deleting...' : 'Delete Team'}
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

export default TeamManagementPanel;
