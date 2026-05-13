import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  aiGetConfig,
  type AIPreview,
  aiRefreshConfig,
  type LocalAiStatus,
  openhumanLocalAiDownload,
  openhumanLocalAiStatus,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const AIPanel = () => {
  const { t } = useT();
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();
  const [aiConfig, setAiConfig] = useState<AIPreview | null>(null);
  const [loading, setLoading] = useState(false);
  const [refreshingComponent, setRefreshingComponent] = useState<'soul' | 'tools' | 'all' | null>(
    null
  );
  const [error, setError] = useState<string>('');
  const [localAiStatus, setLocalAiStatus] = useState<LocalAiStatus | null>(null);
  const localAiRuntimeEnabled = localAiStatus != null && localAiStatus.state !== 'disabled';

  const loadAIPreview = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const config = await aiGetConfig();
      setAiConfig(config);
      if (config.metadata.errors.length > 0) {
        setError(config.metadata.errors.join('; '));
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load AI configuration';
      setError(message);
    } finally {
      setLoading(false);
    }
  }, []);

  const loadLocalAiStatus = useCallback(async () => {
    try {
      const result = await openhumanLocalAiStatus();
      setLocalAiStatus(result.result);
    } catch {
      setLocalAiStatus(null);
    }
  }, []);

  useEffect(() => {
    const initialLoad = window.setTimeout(() => {
      void loadAIPreview();
      void loadLocalAiStatus();
    }, 0);
    const timer = window.setInterval(() => {
      void loadLocalAiStatus();
    }, 5000);
    return () => {
      window.clearTimeout(initialLoad);
      window.clearInterval(timer);
    };
  }, [loadAIPreview, loadLocalAiStatus]);

  const refreshConfig = async (target: 'soul' | 'tools' | 'all') => {
    setRefreshingComponent(target);
    setError('');
    try {
      const config = await aiRefreshConfig();
      setAiConfig(config);
      if (config.metadata.errors.length > 0) {
        setError(config.metadata.errors.join('; '));
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to refresh AI configuration';
      setError(message);
    } finally {
      setRefreshingComponent(null);
    }
  };

  return (
    <div>
      <SettingsHeader
        title={t('settings.aiModels')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <section className="space-y-4">
          <h3 className="text-sm font-semibold text-stone-900">{t('settings.ai.overview')}</h3>
          <p className="text-sm text-stone-500">
            Prompt and markdown orchestration is handled in Rust runtime.
          </p>

          {aiConfig && (
            <div className="bg-stone-50 rounded-lg p-4 border border-stone-200">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    {t('settings.ai.configStatus')}
                  </label>
                  <div className="text-sm text-green-600 font-medium mt-1">
                    {aiConfig.metadata.hasFallbacks
                      ? t('settings.ai.fallbackMode')
                      : t('settings.ai.loadedFromRuntime')}
                  </div>
                </div>
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    {t('settings.ai.loadingDuration')}
                  </label>
                  <div className="text-sm text-primary-600 font-medium mt-1">
                    {aiConfig.metadata.loadingDuration}ms
                  </div>
                </div>
              </div>
            </div>
          )}
        </section>

        <section className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">
              {t('settings.ai.localRuntime')}
            </h3>
            <div className="flex items-center gap-4">
              <button
                onClick={() => navigateToSettings('local-model')}
                className="text-sm text-primary-500 hover:text-primary-600 transition-colors">
                {t('settings.ai.openManager')}
              </button>
              <button
                onClick={async () => {
                  if (!localAiRuntimeEnabled) return;
                  try {
                    setError('');
                    await openhumanLocalAiDownload(true);
                  } catch (err) {
                    const message = err instanceof Error ? err.message : 'Failed to retry download';
                    setError(message);
                  } finally {
                    await loadLocalAiStatus();
                  }
                }}
                className="text-sm text-primary-500 hover:text-primary-600 transition-colors">
                {t('settings.ai.retryDownload')}
                disabled={!localAiRuntimeEnabled}
                className="text-sm text-primary-500 hover:text-primary-600 transition-colors disabled:opacity-50 disabled:hover:text-primary-500">
                Retry Download
              </button>
            </div>
          </div>
          {localAiStatus ? (
            <div className="bg-stone-50 rounded-lg p-4 border border-stone-200 space-y-2">
              <div className="flex items-center justify-between text-sm">
                <span className="text-gray-400">{t('settings.ai.state')}</span>
                <span className="text-primary-600 font-medium">{localAiStatus.state}</span>
              </div>
              <div className="flex items-center justify-between text-sm">
                <span className="text-stone-500">{t('settings.ai.targetModel')}</span>
                <span className="text-green-600 font-medium">{localAiStatus.model_id}</span>
              </div>
              {localAiStatus.download_progress != null && (
                <div className="text-xs text-stone-500">
                  {t('settings.ai.download')}: {(localAiStatus.download_progress * 100).toFixed(0)}%
                </div>
              )}
              {localAiStatus.warning && (
                <div className="text-xs text-amber-700">{localAiStatus.warning}</div>
              )}
            </div>
          ) : (
            <div className="text-sm text-stone-400">{t('settings.ai.localModelUnavailable')}</div>
          )}
        </section>

        <section className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">{t('settings.ai.soulConfig')}</h3>
            <button
              onClick={() => refreshConfig('soul')}
              className="text-sm text-primary-500 hover:text-primary-600 transition-colors disabled:opacity-50"
              disabled={refreshingComponent === 'soul'}>
              {refreshingComponent === 'soul'
                ? t('settings.ai.refreshing')
                : t('settings.ai.refreshSoul')}
            </button>
          </div>

          {loading && (
            <div className="text-sm text-stone-500 animate-pulse">
              {t('settings.ai.loadingSoul')}
            </div>
          )}

          {error && (
            <div className="bg-red-50 border border-red-300 rounded-lg p-3">
              <div className="text-sm text-red-600">{error}</div>
            </div>
          )}

          {aiConfig && (
            <div className="bg-stone-50 rounded-lg p-4 border border-stone-200 space-y-3">
              <div>
                <label className="text-xs text-stone-500 uppercase tracking-wide">
                  {t('settings.ai.identity')}
                </label>
                <div className="text-sm text-green-600 font-medium mt-1">{aiConfig.soul.name}</div>
                <div className="text-xs text-gray-300 mt-1">{aiConfig.soul.description}</div>
              </div>

              {aiConfig.soul.personalityPreview.length > 0 && (
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    {t('settings.ai.personality')}
                  </label>
                  <div className="text-xs text-stone-600 mt-1 leading-relaxed">
                    {aiConfig.soul.personalityPreview.join(' • ')}
                  </div>
                </div>
              )}

              {aiConfig.soul.safetyRulesPreview.length > 0 && (
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    {t('settings.ai.safetyRules')}
                  </label>
                  <div className="text-xs text-yellow-700 mt-1 leading-relaxed">
                    {aiConfig.soul.safetyRulesPreview.join(' • ')}
                  </div>
                </div>
              )}

              <div className="flex items-center justify-between pt-2 border-t border-stone-200">
                <div className="text-xs text-stone-500">
                  {t('settings.ai.source')}: {aiConfig.metadata.sources.soul}
                </div>
                <div className="text-xs text-stone-500">
                  {t('settings.ai.loaded')}: {new Date(aiConfig.soul.loadedAt).toLocaleTimeString()}
                </div>
              </div>
            </div>
          )}
        </section>

        <section className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">{t('settings.ai.toolsConfig')}</h3>
            <button
              onClick={() => refreshConfig('tools')}
              className="text-sm text-primary-500 hover:text-primary-600 transition-colors disabled:opacity-50"
              disabled={refreshingComponent === 'tools'}>
              {refreshingComponent === 'tools'
                ? t('settings.ai.refreshing')
                : t('settings.ai.refreshTools')}
            </button>
          </div>

          {aiConfig && (
            <div className="bg-stone-50 rounded-lg p-4 border border-stone-200 space-y-3">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    {t('settings.ai.toolsAvailable')}
                  </label>
                  <div className="text-sm text-green-600 font-medium mt-1">
                    {aiConfig.tools.totalTools} {t('settings.ai.tools')}
                  </div>
                </div>
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    {t('settings.ai.activeSkills')}
                  </label>
                  <div className="text-sm text-green-600 font-medium mt-1">
                    {aiConfig.tools.activeSkills} {t('settings.ai.skills')}
                  </div>
                </div>
              </div>

              {aiConfig.tools.skillsPreview.length > 0 && (
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    {t('settings.ai.skillsOverview')}
                  </label>
                  <div className="text-xs text-stone-600 mt-1 leading-relaxed">
                    {aiConfig.tools.skillsPreview.join(' • ')}
                  </div>
                </div>
              )}

              <div className="flex items-center justify-between pt-2 border-t border-stone-200">
                <div className="text-xs text-stone-500">
                  {t('settings.ai.source')}: {aiConfig.metadata.sources.tools}
                </div>
                <div className="text-xs text-stone-500">
                  {t('settings.ai.loaded')}:{' '}
                  {new Date(aiConfig.tools.loadedAt).toLocaleTimeString()}
                </div>
              </div>
            </div>
          )}
        </section>

        <section className="space-y-4">
          <div className="flex items-center justify-center">
            <button
              onClick={() => refreshConfig('all')}
              className="px-4 py-2 bg-primary-600 hover:bg-primary-700 text-white text-sm font-medium rounded-lg transition-colors disabled:opacity-50"
              disabled={refreshingComponent === 'all'}>
              {refreshingComponent === 'all'
                ? t('settings.ai.refreshingAll')
                : t('settings.ai.refreshAll')}
            </button>
          </div>
        </section>
      </div>
    </div>
  );
};

export default AIPanel;
