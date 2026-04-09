import { useEffect, useState } from 'react';

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
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();
  const [aiConfig, setAiConfig] = useState<AIPreview | null>(null);
  const [loading, setLoading] = useState(false);
  const [refreshingComponent, setRefreshingComponent] = useState<'soul' | 'tools' | 'all' | null>(
    null
  );
  const [error, setError] = useState<string>('');
  const [localAiStatus, setLocalAiStatus] = useState<LocalAiStatus | null>(null);

  useEffect(() => {
    loadAIPreview();
    void loadLocalAiStatus();
    const timer = setInterval(() => {
      void loadLocalAiStatus();
    }, 5000);
    return () => clearInterval(timer);
  }, []);

  const loadAIPreview = async () => {
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
  };

  const loadLocalAiStatus = async () => {
    try {
      const result = await openhumanLocalAiStatus();
      setLocalAiStatus(result.result);
    } catch {
      setLocalAiStatus(null);
    }
  };

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
        title="AI Configuration"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <section className="space-y-4">
          <h3 className="text-sm font-semibold text-stone-900">AI System Overview</h3>
          <p className="text-sm text-stone-500">
            Prompt and markdown orchestration is handled in Rust runtime.
          </p>

          {aiConfig && (
            <div className="bg-stone-50 rounded-lg p-4 border border-stone-200">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    Configuration Status
                  </label>
                  <div className="text-sm text-green-600 font-medium mt-1">
                    {aiConfig.metadata.hasFallbacks ? 'Fallback Mode' : 'Loaded from Runtime'}
                  </div>
                </div>
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    Loading Duration
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
            <h3 className="text-sm font-semibold text-stone-900">Local Model Runtime</h3>
            <div className="flex items-center gap-4">
              <button
                onClick={() => navigateToSettings('local-model')}
                className="text-sm text-primary-500 hover:text-primary-600 transition-colors">
                Open Manager
              </button>
              <button
                onClick={async () => {
                  await openhumanLocalAiDownload(true);
                  await loadLocalAiStatus();
                }}
                className="text-sm text-primary-500 hover:text-primary-600 transition-colors">
                Retry Download
              </button>
            </div>
          </div>
          {localAiStatus ? (
            <div className="bg-stone-50 rounded-lg p-4 border border-stone-200 space-y-2">
              <div className="flex items-center justify-between text-sm">
                <span className="text-gray-400">State</span>
                <span className="text-primary-600 font-medium">{localAiStatus.state}</span>
              </div>
              <div className="flex items-center justify-between text-sm">
                <span className="text-stone-500">Target Model</span>
                <span className="text-green-600 font-medium">{localAiStatus.model_id}</span>
              </div>
              {localAiStatus.download_progress != null && (
                <div className="text-xs text-stone-500">
                  Download: {(localAiStatus.download_progress * 100).toFixed(0)}%
                </div>
              )}
              {localAiStatus.warning && (
                <div className="text-xs text-amber-700">{localAiStatus.warning}</div>
              )}
            </div>
          ) : (
            <div className="text-sm text-stone-400">Local model status unavailable.</div>
          )}
        </section>

        <section className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">SOUL Persona Configuration</h3>
            <button
              onClick={() => refreshConfig('soul')}
              className="text-sm text-primary-500 hover:text-primary-600 transition-colors disabled:opacity-50"
              disabled={refreshingComponent === 'soul'}>
              {refreshingComponent === 'soul' ? 'Refreshing...' : 'Refresh SOUL'}
            </button>
          </div>

          {loading && (
            <div className="text-sm text-stone-500 animate-pulse">
              Loading SOUL configuration...
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
                <label className="text-xs text-stone-500 uppercase tracking-wide">Identity</label>
                <div className="text-sm text-green-600 font-medium mt-1">{aiConfig.soul.name}</div>
                <div className="text-xs text-gray-300 mt-1">{aiConfig.soul.description}</div>
              </div>

              {aiConfig.soul.personalityPreview.length > 0 && (
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    Personality
                  </label>
                  <div className="text-xs text-stone-600 mt-1 leading-relaxed">
                    {aiConfig.soul.personalityPreview.join(' • ')}
                  </div>
                </div>
              )}

              {aiConfig.soul.safetyRulesPreview.length > 0 && (
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    Safety Rules
                  </label>
                  <div className="text-xs text-yellow-700 mt-1 leading-relaxed">
                    {aiConfig.soul.safetyRulesPreview.join(' • ')}
                  </div>
                </div>
              )}

              <div className="flex items-center justify-between pt-2 border-t border-stone-200">
                <div className="text-xs text-stone-500">
                  Source: {aiConfig.metadata.sources.soul}
                </div>
                <div className="text-xs text-stone-500">
                  Loaded: {new Date(aiConfig.soul.loadedAt).toLocaleTimeString()}
                </div>
              </div>
            </div>
          )}
        </section>

        <section className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">TOOLS Configuration</h3>
            <button
              onClick={() => refreshConfig('tools')}
              className="text-sm text-primary-500 hover:text-primary-600 transition-colors disabled:opacity-50"
              disabled={refreshingComponent === 'tools'}>
              {refreshingComponent === 'tools' ? 'Refreshing...' : 'Refresh TOOLS'}
            </button>
          </div>

          {aiConfig && (
            <div className="bg-stone-50 rounded-lg p-4 border border-stone-200 space-y-3">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    Tools Available
                  </label>
                  <div className="text-sm text-green-600 font-medium mt-1">
                    {aiConfig.tools.totalTools} tools
                  </div>
                </div>
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    Active Skills
                  </label>
                  <div className="text-sm text-green-600 font-medium mt-1">
                    {aiConfig.tools.activeSkills} skills
                  </div>
                </div>
              </div>

              {aiConfig.tools.skillsPreview.length > 0 && (
                <div>
                  <label className="text-xs text-stone-500 uppercase tracking-wide">
                    Skills Overview
                  </label>
                  <div className="text-xs text-stone-600 mt-1 leading-relaxed">
                    {aiConfig.tools.skillsPreview.join(' • ')}
                  </div>
                </div>
              )}

              <div className="flex items-center justify-between pt-2 border-t border-stone-200">
                <div className="text-xs text-stone-500">
                  Source: {aiConfig.metadata.sources.tools}
                </div>
                <div className="text-xs text-stone-500">
                  Loaded: {new Date(aiConfig.tools.loadedAt).toLocaleTimeString()}
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
              {refreshingComponent === 'all' ? 'Refreshing All...' : 'Refresh All AI Configuration'}
            </button>
          </div>
        </section>
      </div>
    </div>
  );
};

export default AIPanel;
