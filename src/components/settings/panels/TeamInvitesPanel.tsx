import { useEffect, useState } from 'react';

import { teamApi } from '../../../services/api/teamApi';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { fetchInvites } from '../../../store/teamSlice';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const TeamInvitesPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const dispatch = useAppDispatch();
  const user = useAppSelector(state => state.user.user);
  const { teams, invites } = useAppSelector(state => state.team);

  const activeTeamId = user?.activeTeamId;
  const activeTeam = teams.find(t => t.team._id === activeTeamId);
  const isAdmin = activeTeam?.role === 'ADMIN';

  const [isGenerating, setIsGenerating] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [revokingId, setRevokingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (activeTeamId) {
      dispatch(fetchInvites(activeTeamId));
    }
  }, [activeTeamId, dispatch]);

  const handleGenerate = async () => {
    if (!activeTeamId) return;
    setIsGenerating(true);
    setError(null);
    try {
      await teamApi.createInvite(activeTeamId);
      dispatch(fetchInvites(activeTeamId));
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

  const handleRevoke = async (inviteId: string) => {
    if (!activeTeamId) return;
    setRevokingId(inviteId);
    setError(null);
    try {
      await teamApi.revokeInvite(activeTeamId, inviteId);
      dispatch(fetchInvites(activeTeamId));
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

  return (
    <div className="overflow-hidden flex flex-col h-full">
      <SettingsHeader title="Invites" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto">
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

          {/* Invites list */}
          {invites.length > 0 ? (
            <div className="space-y-2">
              {invites.map(invite => {
                const expired = isExpired(invite.expiresAt);
                return (
                  <div
                    key={invite._id}
                    className={`rounded-xl border p-3 ${
                      expired
                        ? 'border-stone-700/30 bg-stone-800/20 opacity-60'
                        : 'border-stone-700/50 bg-stone-800/40'
                    }`}>
                    <div className="flex items-center justify-between mb-2">
                      {/* Code */}
                      <code className="text-sm font-mono text-white bg-stone-900/60 px-2 py-1 rounded-lg">
                        {invite.code}
                      </code>
                      <div className="flex items-center gap-1.5">
                        {/* Copy */}
                        <button
                          onClick={() => handleCopy(invite.code, invite._id)}
                          className="p-1.5 rounded-lg text-stone-400 hover:text-white hover:bg-stone-700/50 transition-colors"
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
                        {/* Revoke */}
                        {isAdmin && !expired && (
                          <button
                            onClick={() => handleRevoke(invite._id)}
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
                        {expired
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
        </div>
      </div>
    </div>
  );
};

export default TeamInvitesPanel;
