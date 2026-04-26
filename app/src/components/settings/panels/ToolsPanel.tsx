import { useEffect, useRef, useState } from 'react';

import { useCoreState } from '../../../providers/CoreStateProvider';
import {
  CATEGORY_DESCRIPTIONS,
  getDefaultEnabledTools,
  getEnabledRustToolNames,
  getToolsByCategory,
  TOOL_CATEGORIES,
} from '../../../utils/toolDefinitions';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const ToolsPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { snapshot, setOnboardingTasks } = useCoreState();
  const toolsByCategory = getToolsByCategory();

  const [enabled, setEnabled] = useState<Record<string, boolean>>({});
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<'idle' | 'saved' | 'error'>('idle');
  // Prevents the useEffect from re-initializing state immediately after a save
  // (the core state update triggers a re-render before the ref resets).
  const savingRef = useRef(false);

  const onboardingTasks = snapshot.localState.onboardingTasks;

  // Initialise toggle state from core state (persisted) or defaults.
  useEffect(() => {
    if (savingRef.current) return;
    const persisted = onboardingTasks?.enabledTools;
    const enabledList = persisted && persisted.length > 0 ? persisted : getDefaultEnabledTools();
    const map: Record<string, boolean> = {};
    for (const cat of TOOL_CATEGORIES) {
      for (const tool of toolsByCategory[cat]) {
        map[tool.id] = enabledList.includes(tool.id);
      }
    }
    setEnabled(map);
  }, [onboardingTasks?.enabledTools, toolsByCategory]);

  const toggle = (toolId: string) => {
    setEnabled(prev => ({ ...prev, [toolId]: !prev[toolId] }));
    setDirty(true);
  };

  const handleSave = async () => {
    setSaving(true);
    savingRef.current = true;
    try {
      const enabledIds = Object.entries(enabled)
        .filter(([, v]) => v)
        .map(([k]) => k);

      // Expand UI toggle IDs to the Rust tool names the session builder filters on.
      const enabledTools = getEnabledRustToolNames(enabledIds);

      await setOnboardingTasks({
        accessibilityPermissionGranted: onboardingTasks?.accessibilityPermissionGranted ?? false,
        localModelConsentGiven: onboardingTasks?.localModelConsentGiven ?? false,
        localModelDownloadStarted: onboardingTasks?.localModelDownloadStarted ?? false,
        enabledTools,
        connectedSources: onboardingTasks?.connectedSources ?? [],
        updatedAtMs: Date.now(),
      });
      setDirty(false);
      setSaveStatus('saved');
      setTimeout(() => setSaveStatus('idle'), 3000);
    } catch (err) {
      console.warn('[ToolsPanel] Failed to save tool preferences:', err);
      setSaveStatus('error');
    } finally {
      setSaving(false);
      setTimeout(() => {
        savingRef.current = false;
      }, 500);
    }
  };

  return (
    <div>
      <SettingsHeader
        title="Tools"
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <p className="text-stone-500 text-sm">
          Choose which capabilities OpenHuman can use on your behalf.
        </p>

        <div className="max-h-[420px] overflow-y-auto pr-1 space-y-4">
          {TOOL_CATEGORIES.map(category => {
            const tools = toolsByCategory[category];
            if (tools.length === 0) return null;
            return (
              <div key={category}>
                <div className="mb-2">
                  <h2 className="text-xs font-semibold uppercase tracking-wide text-stone-500">
                    {category}
                  </h2>
                  <p className="text-xs text-stone-400">{CATEGORY_DESCRIPTIONS[category]}</p>
                </div>
                <div className="space-y-1">
                  {tools.map(tool => (
                    <button
                      key={tool.id}
                      type="button"
                      onClick={() => toggle(tool.id)}
                      className="w-full flex items-center justify-between p-2.5 rounded-xl border border-stone-200 bg-white hover:border-stone-300 transition-colors text-left">
                      <div className="min-w-0 flex-1">
                        <span className="text-sm font-medium text-stone-900">
                          {tool.displayName}
                        </span>
                        <p className="text-xs text-stone-500 mt-0.5">{tool.description}</p>
                      </div>
                      <div
                        className={`ml-3 flex-shrink-0 w-9 h-5 rounded-full transition-colors relative ${
                          enabled[tool.id] ? 'bg-sage-500' : 'bg-stone-200'
                        }`}>
                        <div
                          className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
                            enabled[tool.id] ? 'translate-x-4' : 'translate-x-0.5'
                          }`}
                        />
                      </div>
                    </button>
                  ))}
                </div>
              </div>
            );
          })}
        </div>

        {dirty && (
          <button
            type="button"
            onClick={handleSave}
            disabled={saving}
            className="mt-4 w-full py-2 rounded-xl bg-primary-600 text-white text-sm font-medium hover:bg-primary-500 transition-colors disabled:opacity-50">
            {saving ? 'Saving...' : 'Save Changes'}
          </button>
        )}
        {saveStatus === 'saved' && (
          <p className="text-xs text-center text-green-600 mt-1">Preferences saved</p>
        )}
        {saveStatus === 'error' && (
          <p className="text-xs text-center text-red-500 mt-1">
            Failed to save preferences. Try again.
          </p>
        )}
      </div>
    </div>
  );
};

export default ToolsPanel;
