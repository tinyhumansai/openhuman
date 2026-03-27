import { useEffect, useState } from 'react';

import { aiGetConfig, type AIPreview, aiRefreshConfig } from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const AIPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const [aiConfig, setAiConfig] = useState<AIPreview | null>(null);
  const [loading, setLoading] = useState(false);
  const [refreshingComponent, setRefreshingComponent] = useState<'soul' | 'tools' | 'all' | null>(
    null
  );
  const [error, setError] = useState<string>('');

  useEffect(() => {
    loadAIPreview();
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
    <div className="h-full flex flex-col">
      <SettingsHeader title="AI Configuration" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto px-6 pb-10 space-y-6">
        <section className="space-y-4">
          <h3 className="text-lg font-semibold text-white">AI System Overview</h3>
          <p className="text-sm text-gray-400">
            Prompt and markdown orchestration is handled in Rust runtime.
          </p>

          {aiConfig && (
            <div className="bg-gray-900 rounded-lg p-4 border border-gray-700">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="text-xs text-gray-400 uppercase tracking-wide">
                    Configuration Status
                  </label>
                  <div className="text-sm text-green-400 font-medium mt-1">
                    {aiConfig.metadata.hasFallbacks ? 'Fallback Mode' : 'Loaded from Runtime'}
                  </div>
                </div>
                <div>
                  <label className="text-xs text-gray-400 uppercase tracking-wide">
                    Loading Duration
                  </label>
                  <div className="text-sm text-blue-400 font-medium mt-1">
                    {aiConfig.metadata.loadingDuration}ms
                  </div>
                </div>
              </div>
            </div>
          )}
        </section>

        <section className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-lg font-semibold text-white">SOUL Persona Configuration</h3>
            <button
              onClick={() => refreshConfig('soul')}
              className="text-sm text-blue-400 hover:text-blue-300 transition-colors disabled:opacity-50"
              disabled={refreshingComponent === 'soul'}>
              {refreshingComponent === 'soul' ? 'Refreshing...' : 'Refresh SOUL'}
            </button>
          </div>

          {loading && (
            <div className="text-sm text-gray-400 animate-pulse">Loading SOUL configuration...</div>
          )}

          {error && (
            <div className="bg-red-500/10 border border-red-500/40 rounded-lg p-3">
              <div className="text-sm text-red-200">{error}</div>
            </div>
          )}

          {aiConfig && (
            <div className="bg-gray-900 rounded-lg p-4 border border-gray-700 space-y-3">
              <div>
                <label className="text-xs text-gray-400 uppercase tracking-wide">Identity</label>
                <div className="text-sm text-green-400 font-medium mt-1">{aiConfig.soul.name}</div>
                <div className="text-xs text-gray-300 mt-1">{aiConfig.soul.description}</div>
              </div>

              {aiConfig.soul.personalityPreview.length > 0 && (
                <div>
                  <label className="text-xs text-gray-400 uppercase tracking-wide">
                    Personality
                  </label>
                  <div className="text-xs text-gray-300 mt-1 leading-relaxed">
                    {aiConfig.soul.personalityPreview.join(' • ')}
                  </div>
                </div>
              )}

              {aiConfig.soul.safetyRulesPreview.length > 0 && (
                <div>
                  <label className="text-xs text-gray-400 uppercase tracking-wide">
                    Safety Rules
                  </label>
                  <div className="text-xs text-yellow-300 mt-1 leading-relaxed">
                    {aiConfig.soul.safetyRulesPreview.join(' • ')}
                  </div>
                </div>
              )}

              <div className="flex items-center justify-between pt-2 border-t border-gray-700">
                <div className="text-xs text-gray-400">
                  Source: {aiConfig.metadata.sources.soul}
                </div>
                <div className="text-xs text-gray-400">
                  Loaded: {new Date(aiConfig.soul.loadedAt).toLocaleTimeString()}
                </div>
              </div>
            </div>
          )}
        </section>

        <section className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="text-lg font-semibold text-white">TOOLS Configuration</h3>
            <button
              onClick={() => refreshConfig('tools')}
              className="text-sm text-blue-400 hover:text-blue-300 transition-colors disabled:opacity-50"
              disabled={refreshingComponent === 'tools'}>
              {refreshingComponent === 'tools' ? 'Refreshing...' : 'Refresh TOOLS'}
            </button>
          </div>

          {aiConfig && (
            <div className="bg-gray-900 rounded-lg p-4 border border-gray-700 space-y-3">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="text-xs text-gray-400 uppercase tracking-wide">
                    Tools Available
                  </label>
                  <div className="text-sm text-green-400 font-medium mt-1">
                    {aiConfig.tools.totalTools} tools
                  </div>
                </div>
                <div>
                  <label className="text-xs text-gray-400 uppercase tracking-wide">
                    Active Skills
                  </label>
                  <div className="text-sm text-green-400 font-medium mt-1">
                    {aiConfig.tools.activeSkills} skills
                  </div>
                </div>
              </div>

              {aiConfig.tools.skillsPreview.length > 0 && (
                <div>
                  <label className="text-xs text-gray-400 uppercase tracking-wide">
                    Skills Overview
                  </label>
                  <div className="text-xs text-gray-300 mt-1 leading-relaxed">
                    {aiConfig.tools.skillsPreview.join(' • ')}
                  </div>
                </div>
              )}

              <div className="flex items-center justify-between pt-2 border-t border-gray-700">
                <div className="text-xs text-gray-400">
                  Source: {aiConfig.metadata.sources.tools}
                </div>
                <div className="text-xs text-gray-400">
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
              className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors disabled:opacity-50"
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
