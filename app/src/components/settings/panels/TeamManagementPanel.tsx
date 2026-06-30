import { useEffect, useRef, useState } from 'react';
import { useParams } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import { teamApi } from '../../../services/api/teamApi';
import Button from '../../ui/Button';
import SettingsMenuItem from '../components/SettingsMenuItem';
import { SettingsSection, SettingsTextField } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';

const TeamManagementPanel = () => {
  const { t } = useT();
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
          : t('team.failedToUpdate')
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
          : t('team.failedToDelete')
      );
      setIsDeleting(false);
    }
  };

  if (!teamEntry) {
    return (
      <SettingsPanel
        title={t('team.management')}
        description={t('pages.settings.account.teamDesc')}>
        <div className="flex-1 flex items-center justify-center">
          <p className="text-sm text-content-muted">{t('team.notFound')}</p>
        </div>
      </SettingsPanel>
    );
  }

  if (!isAdmin) {
    return (
      <SettingsPanel
        title={t('team.management')}
        description={t('pages.settings.account.teamDesc')}>
        <div className="flex-1 flex items-center justify-center">
          <p className="text-sm text-content-muted">{t('team.accessDenied')}</p>
        </div>
      </SettingsPanel>
    );
  }

  const { team } = teamEntry;

  return (
    <SettingsPanel
      title={t('team.manageTitle').replace('{name}', team.name)}
      description={t('pages.settings.account.teamDesc')}>
      <>
        {/* Team Info */}
        <SettingsSection>
          <div className="flex items-center gap-3 px-4 py-3">
            <div className="w-10 h-10 rounded-lg bg-surface-strong flex items-center justify-center">
              <span className="text-sm font-semibold text-content-secondary">
                {team.name.charAt(0).toUpperCase()}
              </span>
            </div>
            <div>
              <h3 className="text-sm font-semibold text-content">{team.name}</h3>
              <p className="text-xs text-content-muted">
                {t('team.planCreated')
                  .replace('{plan}', team.subscription.plan)
                  .replace('{date}', new Date(team.createdAt).toLocaleDateString())}
              </p>
            </div>
          </div>
        </SettingsSection>

        {/* Management Options */}
        <SettingsSection title={t('team.management')}>
          {/* Members */}
          <SettingsMenuItem
            icon={
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 4.354a4 4 0 110 5.292M15 21H3v-1a6 6 0 0112 0v1zm0 0h6v-1a6 6 0 00-9-5.197m13.5-9a2.5 2.5 0 11-5 0 2.5 2.5 0 015 0z"
                />
              </svg>
            }
            title={t('team.members')}
            description={t('team.membersDesc')}
            onClick={() => navigateToSettings(`team/manage/${teamId}/members`)}
            testId="settings-nav-team-members"
            isFirst={true}
            isLast={false}
          />

          {/* Invites */}
          <SettingsMenuItem
            icon={
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
                />
              </svg>
            }
            title={t('team.invites')}
            description={t('team.invitesDesc')}
            onClick={() => navigateToSettings(`team/manage/${teamId}/invites`)}
            testId="settings-nav-team-invites"
            isFirst={false}
            isLast={false}
          />

          {/* Edit Team Settings */}
          <SettingsMenuItem
            icon={
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
                />
              </svg>
            }
            title={t('team.settings')}
            description={t('team.settingsDesc')}
            onClick={handleEditTeam}
            testId="settings-nav-team-settings"
            isFirst={false}
            isLast={!teamEntry?.team.isPersonal ? false : true}
          />

          {/* Delete Team */}
          {!teamEntry?.team.isPersonal && (
            <SettingsMenuItem
              icon={
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
              }
              title={t('team.delete')}
              description={t('team.deleteDesc')}
              onClick={() => setIsDeleteModalOpen(true)}
              testId="settings-nav-team-delete"
              isFirst={false}
              isLast={true}
              dangerous
            />
          )}
        </SettingsSection>

        {/* Edit Team Modal */}
        {isEditModalOpen && (
          <div className="fixed inset-0 bg-neutral-900/40 flex items-center justify-center z-50 p-4">
            <div className="bg-surface rounded-2xl p-6 w-full max-w-md border border-line">
              <h3 className="text-sm font-semibold text-content mb-4">{t('team.editSettings')}</h3>

              {error && (
                <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                  <p className="text-xs text-coral-600 dark:text-coral-300">{error}</p>
                </div>
              )}

              <div className="space-y-4">
                <div>
                  <label className="block text-sm font-medium text-content-secondary mb-2">
                    {t('team.teamName')}
                  </label>
                  <SettingsTextField
                    value={editTeamName}
                    onChange={e => setEditTeamName(e.target.value)}
                    onKeyDown={e => e.key === 'Enter' && void handleUpdateTeam()}
                    placeholder={t('team.enterName')}
                    aria-label={t('team.teamName')}
                    inputSize="sm"
                    className="w-full"
                  />
                </div>

                <div className="flex gap-2 pt-2">
                  <Button
                    type="button"
                    variant="secondary"
                    size="md"
                    className="flex-1"
                    onClick={() => setIsEditModalOpen(false)}
                    disabled={isUpdating}>
                    {t('common.cancel')}
                  </Button>
                  <Button
                    type="button"
                    variant="primary"
                    size="md"
                    className="flex-1"
                    onClick={() => void handleUpdateTeam()}
                    disabled={isUpdating || !editTeamName.trim()}>
                    {isUpdating ? t('team.saving') : t('team.saveChanges')}
                  </Button>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* Delete Team Modal */}
        {isDeleteModalOpen && (
          <div className="fixed inset-0 bg-neutral-900/40 flex items-center justify-center z-50 p-4">
            <div className="bg-surface rounded-2xl p-6 w-full max-w-md border border-line">
              <h3 className="text-sm font-semibold text-content mb-4">{t('team.delete')}</h3>

              {error && (
                <div className="rounded-xl bg-coral-500/10 border border-coral-500/20 p-3 mb-4">
                  <p className="text-xs text-coral-600 dark:text-coral-300">{error}</p>
                </div>
              )}

              <div className="space-y-4">
                <div className="text-sm text-content-muted">
                  <p>{t('team.confirmDelete').replace('{name}', teamEntry?.team.name ?? '')}</p>
                  <p className="mt-2 text-coral-400">{t('team.deleteWarning')}</p>
                </div>

                <div className="flex gap-2 pt-2">
                  <Button
                    type="button"
                    variant="secondary"
                    size="md"
                    className="flex-1"
                    onClick={() => setIsDeleteModalOpen(false)}
                    disabled={isDeleting}>
                    {t('common.cancel')}
                  </Button>
                  <Button
                    type="button"
                    variant="primary"
                    tone="danger"
                    size="md"
                    className="flex-1"
                    onClick={() => void handleDeleteTeam()}
                    disabled={isDeleting}>
                    {isDeleting ? t('team.deleting') : t('team.delete')}
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

export default TeamManagementPanel;
