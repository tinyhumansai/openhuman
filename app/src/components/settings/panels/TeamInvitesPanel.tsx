import { useEffect, useState } from 'react';
import { useLocation, useParams } from 'react-router-dom';

import { useCoreState } from '../../../providers/CoreStateProvider';
import { teamApi } from '../../../services/api/teamApi';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const TeamInvitesPanel = () => {
  const { teamId } = useParams<{ teamId: string }>();
  const location = useLocation();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { snapshot, teams, teamInvitesById, refreshTeamInvites } = useCoreState();
  const user = snapshot.currentUser;

  // Check if we're in team management context (has teamId in URL)
  const isInManagementContext = location.pathname.includes('/team/manage/');
  const currentTeamId = isInManagementContext ? teamId : user?.activeTeamId;
  const currentTeam = teams.find(t => t.team._id === currentTeamId);
  const isAdmin = currentTeam?.role.toUpperCase() === 'ADMIN';
  const invites = currentTeamId ? (teamInvitesById[currentTeamId] ?? []) : [];

  const [isGenerating, setIsGenerating] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [revokingId, setRevokingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoadingInvites, setIsLoadingInvites] = useState(false);

  // Confirmation modal state
  const [inviteToRevoke, setInviteToRevoke] = useState<{ id: string; code: string } | null>(null);

  useEffect(() => {
    if (!currentTeamId) return;
    setIsLoadingInvites(true);
    void refreshTeamInvites(currentTeamId).finally(() => setIsLoadingInvites(false));
  }, [currentTeamId, refreshTeamInvites]);

  const handleGenerate = async () => {
    if (!currentTeamId) return;
    setIsGenerating(true);
    setError(null);
    try {
      await teamApi.createInvite(currentTeamId);
      await refreshTeamInvites(currentTeamId);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Failed to generate invite'
      );
    } finally {
      setIsGenerating(false);
    }
  };

  const handleCopy = async (code: string, inviteId: string) => {
    try {
      await navigator.clipboard.writeText(code);
      setCopiedId(inviteId);
      setTimeout(() => setCopiedId(null), 2000);
    } catch {
      // Fallback: select text
    }
  };

  const handleRevoke = (inviteId: string, inviteCode: string) => {
    // Show confirmation modal for revoking invites
    setInviteToRevoke({ id: inviteId, code: inviteCode });
  };

  const confirmRevokeInvite = async () => {
    if (!inviteToRevoke || !currentTeamId) return;

    setRevokingId(inviteToRevoke.id);
    setError(null);

    try {
      await teamApi.revokeInvite(currentTeamId, inviteToRevoke.id);
      await refreshTeamInvites(currentTeamId);
      setInviteToRevoke(null);
    } catch (err) {
      setError(
        err && typeof err === 'object' && 'error' in err
          ? String(err.error)
          : 'Failed to revoke invite'
      );
    } finally {
      setRevokingId(null);
    }
  };

  const isExpired = (expiresAt: string) => new Date(expiresAt) < new Date();

  const isUsedUp = (invite: { maxUses: number; currentUses: number }) =>
    invite.maxUses > 0 && invite.currentUses >= invite.maxUses;

  const getInviteStatus = (invite: { expiresAt: string; maxUses: number; currentUses: number }) => {
    if (isExpired(invite.expiresAt)) return 'expired';
    if (isUsedUp(invite)) return 'used';
    return 'active';
  };

  return (
    <div>
      <SettingsHeader
        title="Invites"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4 space-y-4">
          {error && (
            <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3">
              <p className="text-xs text-coral-400">{error}</p>
            </div>
          )}

          {/* Generate button */}
          {isAdmin && (
            <button
              onClick={handleGenerate}
              disabled={isGenerating}
              className="w-full flex items-center justify-center gap-2 px-4 py-2.5 text-sm font-medium rounded-xl bg-primary-500 hover:bg-primary-600 text-white transition-colors disabled:opacity-50">
              {isGenerating ? (
                <>
                  <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
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
                  Generating...
                </>
              ) : (
                <>
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M12 4v16m8-8H4"
                    />
                  </svg>
                  Generate Invite
                </>
              )}
            </button>
          )}

          {/* Refreshing indicator - only when loading and has existing data */}
          {isLoadingInvites && invites.length > 0 && (
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
              Refreshing invites...
            </div>
          )}

          {/* Invites list */}
          {isLoadingInvites && invites.length === 0 ? (
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
              <span className="ml-3 text-sm text-stone-500">Loading invites...</span>
            </div>
          ) : invites.length > 0 ? (
            <div className="space-y-2">
              {invites.map(invite => {
                const status = getInviteStatus(invite);
                const isInactive = status !== 'active';

                return (
                  <div
                    key={invite._id}
                    className={`rounded-xl border p-3 ${
                      isInactive
                        ? 'border-stone-200 bg-stone-50 opacity-60'
                        : 'border-stone-200 bg-white'
                    }`}>
                    <div className="flex items-center justify-between mb-2">
                      {/* Code with status label */}
                      <div className="flex items-center gap-2">
                        <code
                          className={`text-sm font-mono px-2 py-1 rounded-lg ${
                            isInactive
                              ? 'text-stone-500 bg-stone-100'
                              : 'text-stone-900 bg-stone-200'
                          }`}>
                          {invite.code}
                        </code>
                        {status === 'expired' && (
                          <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-coral-500/20 text-coral-400 border border-coral-500/30">
                            Expired
                          </span>
                        )}
                        {status === 'used' && (
                          <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-amber-500/20 text-amber-400 border border-amber-500/30">
                            Used Up
                          </span>
                        )}
                      </div>
                      <div className="flex items-center gap-1.5">
                        {/* Copy */}
                        <button
                          onClick={() => handleCopy(invite.code, invite._id)}
                          disabled={status !== 'active'}
                          className={`p-1.5 rounded-lg transition-colors ${
                            status === 'active'
                              ? 'text-stone-500 hover:text-stone-900 hover:bg-stone-100'
                              : 'text-stone-600 cursor-not-allowed'
                          }`}
                          aria-label="Copy invite code">
                          {copiedId === invite._id ? (
                            <svg
                              className="w-4 h-4 text-sage-400"
                              fill="none"
                              stroke="currentColor"
                              viewBox="0 0 24 24">
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                strokeWidth={2}
                                d="M5 13l4 4L19 7"
                              />
                            </svg>
                          ) : (
                            <svg
                              className="w-4 h-4"
                              fill="none"
                              stroke="currentColor"
                              viewBox="0 0 24 24">
                              <path
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                strokeWidth={2}
                                d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                              />
                            </svg>
                          )}
                        </button>
                        {/* Revoke - only for active invites */}
                        {isAdmin && status === 'active' && (
                          <button
                            onClick={() => handleRevoke(invite._id, invite.code)}
                            disabled={revokingId === invite._id}
                            className="p-1.5 rounded-lg text-stone-500 hover:text-coral-400 hover:bg-coral-500/10 transition-colors disabled:opacity-50"
                            aria-label="Revoke invite">
                            <svg
                              className="w-4 h-4"
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
                          </button>
                        )}
                      </div>
                    </div>
                    <div className="flex items-center gap-3 text-xs text-stone-500">
                      <span>
                        Uses: {invite.currentUses}
                        {invite.maxUses > 0 ? `/${invite.maxUses}` : ''}
                      </span>
                      <span>
                        {status === 'expired'
                          ? 'Expired'
                          : `Expires ${new Date(invite.expiresAt).toLocaleDateString()}`}
                      </span>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="text-center py-8">
              <svg
                className="w-10 h-10 mx-auto text-stone-600 mb-3"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
                />
              </svg>
              <p className="text-sm text-stone-500">No invites yet</p>
              <p className="text-xs text-stone-600 mt-1">
                Generate an invite code to share with others
              </p>
            </div>
          )}

          {/* Revoke Invite Confirmation Modal */}
          {inviteToRevoke && (
            <div className="fixed inset-0 bg-stone-900/50 flex items-center justify-center z-50 p-4">
              <div className="bg-white rounded-2xl p-6 w-full max-w-md border border-stone-200">
                <h3 className="text-sm font-semibold text-stone-900 mb-4">Revoke Invite Code</h3>

                {error && (
                  <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                    <p className="text-xs text-coral-400">{error}</p>
                  </div>
                )}

                <div className="space-y-4">
                  <div className="text-sm text-stone-400">
                    <p>
                      Are you sure you want to revoke the invite code{' '}
                      <code className="text-stone-900 bg-stone-100 px-1.5 py-0.5 rounded font-mono text-xs">
                        {inviteToRevoke.code}
                      </code>
                      ?
                    </p>
                    <p className="mt-2 text-amber-400">
                      This invite code will no longer be valid and cannot be used to join the team.
                    </p>
                  </div>

                  <div className="flex gap-2 pt-2">
                    <button
                      onClick={() => setInviteToRevoke(null)}
                      disabled={revokingId === inviteToRevoke.id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-stone-100 hover:bg-stone-200 text-stone-700 transition-colors disabled:opacity-50">
                      Cancel
                    </button>
                    <button
                      onClick={confirmRevokeInvite}
                      disabled={revokingId === inviteToRevoke.id}
                      className="flex-1 px-4 py-2 text-sm font-medium rounded-xl bg-coral-500 hover:bg-coral-600 text-white transition-colors disabled:opacity-50">
                      {revokingId === inviteToRevoke.id ? 'Revoking...' : 'Revoke Invite'}
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

export default TeamInvitesPanel;
