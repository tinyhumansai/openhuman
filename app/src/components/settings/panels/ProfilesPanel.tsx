/**
 * ProfilesPanel — Settings > Agent Profiles.
 *
 * Lists agent *profiles* ("flavours"): the active one, built-in defaults, and
 * user-created profiles. Lets the user set the active profile, create a new
 * one, edit, or delete a custom profile. The editor lives at
 * `/settings/profiles/(new|edit/:id)` (`ProfileEditorPage`).
 */
import { useCallback, useEffect, useState } from 'react';
import { LuPlus } from 'react-icons/lu';
import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  deleteAgentProfile,
  loadAgentProfiles,
  selectActiveAgentProfileId,
  selectAgentProfile,
  selectAgentProfiles,
} from '../../../store/agentProfileSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import Button from '../../ui/Button';
import { SettingsEmptyState, SettingsSection } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import { settingsNavState } from '../modal/settingsOverlay';

const ProfilesPanel = () => {
  const { t } = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const dispatch = useAppDispatch();
  const profiles = useAppSelector(selectAgentProfiles);
  const activeId = useAppSelector(selectActiveAgentProfileId);
  const status = useAppSelector(state => state.agentProfiles.status);
  const error = useAppSelector(state => state.agentProfiles.error);
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    void dispatch(loadAgentProfiles());
  }, [dispatch]);

  const setActive = useCallback(
    async (id: string) => {
      setActionError(null);
      try {
        await dispatch(selectAgentProfile(id)).unwrap();
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err));
      }
    },
    [dispatch]
  );

  const remove = useCallback(
    async (id: string) => {
      if (!window.confirm(t('settings.profiles.deleteConfirm'))) return;
      setActionError(null);
      try {
        await dispatch(deleteAgentProfile(id)).unwrap();
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err));
      }
    },
    [dispatch, t]
  );

  return (
    <SettingsPanel
      description={t('settings.profiles.menuDesc')}
      action={
        <Button
          type="button"
          variant="primary"
          size="sm"
          onClick={() => navigate('/settings/profiles/new', settingsNavState(location))}>
          <LuPlus className="h-4 w-4" />
          {t('settings.profiles.new')}
        </Button>
      }>
      <>
        <p className="text-sm text-content-muted">{t('settings.profiles.subtitle')}</p>

        {(actionError || error) && (
          <p className="rounded-md border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:border-coral-500/30 dark:bg-coral-500/10 dark:text-coral-300">
            {actionError || `${t('settings.profiles.loadError')}: ${error}`}
          </p>
        )}

        {profiles.length === 0 ? (
          status === 'loading' ? (
            <div className="flex items-center justify-center py-12 text-content-faint">
              <div className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-ocean-500 border-t-transparent" />
              <span className="text-sm">{t('common.loading')}</span>
            </div>
          ) : (
            <SettingsEmptyState label={t('settings.profiles.empty')} />
          )
        ) : (
          <SettingsSection>
            <ul className="divide-y divide-line-subtle dark:divide-neutral-800">
              {profiles.map(profile => {
                const isActive = profile.id === activeId;
                return (
                  <li
                    key={profile.id}
                    className="flex items-center justify-between gap-3 px-4 py-3">
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-sm font-semibold text-content">
                          {profile.name}
                        </span>
                        {isActive && (
                          <span className="rounded-full bg-sage-100 px-2 py-0.5 text-[10px] font-medium text-sage-700 dark:bg-sage-500/15 dark:text-sage-300">
                            {t('settings.profiles.active')}
                          </span>
                        )}
                        <span className="rounded-full bg-surface-subtle px-2 py-0.5 text-[10px] font-medium text-content-muted">
                          {profile.builtIn
                            ? t('settings.profiles.sourceBuiltIn')
                            : t('settings.profiles.sourceCustom')}
                        </span>
                      </div>
                      {profile.description && (
                        <p className="mt-0.5 truncate text-xs text-content-muted">
                          {profile.description}
                        </p>
                      )}
                    </div>
                    <div className="flex flex-none items-center gap-1.5">
                      {!isActive && (
                        <Button
                          type="button"
                          variant="secondary"
                          size="sm"
                          onClick={() => void setActive(profile.id)}>
                          {t('settings.profiles.setActive')}
                        </Button>
                      )}
                      <Button
                        type="button"
                        variant="secondary"
                        size="sm"
                        onClick={() =>
                          navigate(
                            `/settings/profiles/edit/${profile.id}`,
                            settingsNavState(location)
                          )
                        }>
                        {t('common.edit')}
                      </Button>
                      {!profile.builtIn && (
                        <Button
                          type="button"
                          variant="tertiary"
                          size="sm"
                          onClick={() => void remove(profile.id)}>
                          {t('common.delete')}
                        </Button>
                      )}
                    </div>
                  </li>
                );
              })}
            </ul>
          </SettingsSection>
        )}
      </>
    </SettingsPanel>
  );
};

export default ProfilesPanel;
