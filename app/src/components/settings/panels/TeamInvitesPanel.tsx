import debug from 'debug';
import { useEffect, useState } from 'react';
import { useLocation, useParams } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { teamApi } from '../../../services/api/teamApi';
import { sanitizeError } from '../../../utils/sanitize';
import { CenteredLoadingState, ErrorBanner, InlineLoadingStatus, Spinner } from '../../ui';
import Button from '../../ui/Button';
import { SettingsBadge, SettingsEmptyState, SettingsSection } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const log = debug('core-rpc:error');

const TeamInvitesPanel = () => {
  const { t } = useT();
  const { teamId } = useParams<{ teamId: string }>();
  const location = useLocation();
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
    // `.finally()` alone left this as `void promise(...)`, so any rejection
    // (cold core boot, backend 504, local AbortController timeout) became an
    // unhandled rejection → OPENHUMAN-REACT-12. Swallow into a logged
    // breadcrumb; the user can retry by navigating away and back.
    refreshTeamInvites(currentTeamId)
      .catch(err => {
        log('refreshTeamInvites failed in TeamInvitesPanel: %O', sanitizeError(err));
      })
      .finally(() => setIsLoadingInvites(false));
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
          : t('invites.failedGenerate')
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
          : t('invites.failedRevoke')
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
    <SettingsPanel title={t('team.invites')} description={t('pages.settings.account.teamDesc')}>
      <>
        {error && <ErrorBanner message={error} />}

        {/* Generate button */}
        {isAdmin && (
          <div className="px-1">
            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={() => void handleGenerate()}
              disabled={isGenerating}
              className="w-full">
              {isGenerating ? (
                <>
                  <Spinner className="w-4 h-4" />
                  {t('invites.generating')}
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
                  {t('invites.generate')}
                </>
              )}
            </Button>
          </div>
        )}

        {/* Refreshing indicator - only when loading and has existing data */}
        {isLoadingInvites && invites.length > 0 && (
          <InlineLoadingStatus label={t('invites.refreshing')} />
        )}

        {/* Invites list */}
        {isLoadingInvites && invites.length === 0 ? (
          <CenteredLoadingState label={t('invites.loading')} />
        ) : invites.length > 0 ? (
          <SettingsSection>
            <ul>
              {invites.map(invite => {
                const status = getInviteStatus(invite);
                const isInactive = status !== 'active';

                return (
                  <li
                    key={invite._id}
                    className={`px-4 py-3 border-b border-line-subtle last:border-b-0 ${
                      isInactive ? 'opacity-60' : ''
                    }`}>
                    <div className="flex items-center justify-between mb-2">
                      {/* Code with status label */}
                      <div className="flex items-center gap-2">
                        <code
                          className={`text-sm font-mono px-2 py-1 rounded-lg ${
                            isInactive
                              ? 'text-content-muted bg-surface-subtle'
                              : 'text-content bg-surface-strong'
                          }`}>
                          {invite.code}
                        </code>
                        {status === 'expired' && (
                          <SettingsBadge variant="danger">
                            {t('rewards.referralSection.statusExpired')}
                          </SettingsBadge>
                        )}
                        {status === 'used' && (
                          <SettingsBadge variant="warning">{t('invites.usedUp')}</SettingsBadge>
                        )}
                      </div>
                      <div className="flex items-center gap-1.5">
                        {/* Copy */}
                        <Button
                          type="button"
                          variant="tertiary"
                          size="xs"
                          onClick={() => void handleCopy(invite.code, invite._id)}
                          disabled={status !== 'active'}
                          aria-label={t('invites.copyCodeAria')}>
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
                        </Button>
                        {/* Revoke - only for active invites */}
                        {isAdmin && status === 'active' && (
                          <Button
                            type="button"
                            variant="tertiary"
                            size="xs"
                            onClick={() => handleRevoke(invite._id, invite.code)}
                            disabled={revokingId === invite._id}
                            aria-label={t('invites.revokeAria')}
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
                                d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                              />
                            </svg>
                          </Button>
                        )}
                      </div>
                    </div>
                    <div className="flex items-center gap-3 text-xs text-content-muted">
                      <span>
                        {t('invites.uses')
                          .replace('{current}', String(invite.currentUses))
                          .replace('{max}', invite.maxUses > 0 ? `/${invite.maxUses}` : '')}
                      </span>
                      <span>
                        {status === 'expired'
                          ? t('rewards.referralSection.statusExpired')
                          : t('invites.expiresOn').replace(
                              '{date}',
                              new Date(invite.expiresAt).toLocaleDateString()
                            )}
                      </span>
                    </div>
                  </li>
                );
              })}
            </ul>
          </SettingsSection>
        ) : (
          <SettingsSection>
            <SettingsEmptyState label={t('invites.empty')} />
          </SettingsSection>
        )}

        {/* Revoke Invite Confirmation Modal */}
        {inviteToRevoke && (
          <div className="fixed inset-0 bg-neutral-900/50 flex items-center justify-center z-50 p-4">
            <div className="bg-surface rounded-2xl p-6 w-full max-w-md border border-line">
              <h3 className="text-sm font-semibold text-content mb-4">
                {t('invites.revokeTitle')}
              </h3>

              {error && (
                <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                  <p className="text-xs text-coral-400">{error}</p>
                </div>
              )}

              <div className="space-y-4">
                <div className="text-sm text-content-muted">
                  <p>
                    {t('invites.revokePromptPrefix')}{' '}
                    <code className="text-content bg-surface-subtle px-1.5 py-0.5 rounded font-mono text-xs">
                      {inviteToRevoke.code}
                    </code>
                    ?
                  </p>
                  <p className="mt-2 text-amber-400">{t('invites.revokeWarning')}</p>
                </div>

                <div className="flex gap-2 pt-2">
                  <Button
                    type="button"
                    variant="secondary"
                    size="md"
                    className="flex-1"
                    onClick={() => setInviteToRevoke(null)}
                    disabled={revokingId === inviteToRevoke.id}>
                    {t('common.cancel')}
                  </Button>
                  <Button
                    type="button"
                    variant="primary"
                    tone="danger"
                    size="md"
                    className="flex-1"
                    onClick={() => void confirmRevokeInvite()}
                    disabled={revokingId === inviteToRevoke.id}>
                    {revokingId === inviteToRevoke.id
                      ? t('invites.revoking')
                      : t('invites.revokeAction')}
                  </Button>
                </div>
              </div>
            </div>
          </div>
        )}
      </>
    </SettingsPanel>
  );
};

export default TeamInvitesPanel;
