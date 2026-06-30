import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import {
  CATEGORY_DESCRIPTIONS,
  getDefaultEnabledTools,
  getEnabledRustToolNames,
  getToolsByCategory,
  normalizeEnabledToolList,
  TOOL_CATEGORIES,
} from '../../../utils/toolDefinitions';
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import { SettingsRow, SettingsSection, SettingsStatusLine, SettingsSwitch } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

interface ToolsPanelProps {
  /** When true, render without the SettingsHeader chrome (used when embedded
   *  inside the onboarding custom wizard). */
  embedded?: boolean;
}

const ToolsPanel = ({ embedded = false }: ToolsPanelProps = {}) => {
  const { t } = useT();
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
    // normalizeEnabledToolList converts persisted Rust tool names (e.g.
    // "web_search_tool") back to UI toggle IDs ("web_search") so the
    // includes() check below works regardless of what format was saved
    // (fixes #2742: web_search toggle auto-reverts to OFF).
    const enabledList =
      persisted && persisted.length > 0
        ? normalizeEnabledToolList(persisted)
        : getDefaultEnabledTools();
    const map: Record<string, boolean> = {};
    for (const cat of TOOL_CATEGORIES) {
      for (const tool of toolsByCategory[cat]) {
        map[tool.id] = enabledList.includes(tool.id);
      }
    }
    setEnabled(map);
  }, [onboardingTasks?.enabledTools]); // eslint-disable-line react-hooks/exhaustive-deps

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

  const body = (
    <>
      <p className="text-content-muted text-sm">{t('settings.tools.chooseCapabilities')}</p>

      <div className="max-h-[420px] overflow-y-auto pr-1 space-y-4">
        {TOOL_CATEGORIES.map(category => {
          const tools = toolsByCategory[category];
          if (tools.length === 0) return null;
          return (
            <SettingsSection
              key={category}
              title={category}
              description={CATEGORY_DESCRIPTIONS[category]}>
              {tools.map(tool => (
                <SettingsRow
                  key={tool.id}
                  htmlFor={`tool-switch-${tool.id}`}
                  label={tool.displayName}
                  description={tool.description}
                  control={
                    <SettingsSwitch
                      id={`tool-switch-${tool.id}`}
                      checked={Boolean(enabled[tool.id])}
                      onCheckedChange={() => toggle(tool.id)}
                      aria-label={tool.displayName}
                    />
                  }
                />
              ))}
            </SettingsSection>
          );
        })}
      </div>

      {dirty && (
        <Button
          type="button"
          variant="primary"
          size="md"
          className="w-full"
          onClick={() => void handleSave()}
          disabled={saving}>
          {saving ? t('autonomy.statusSaving') : t('settings.tools.saveChanges')}
        </Button>
      )}

      <SettingsStatusLine
        saving={false}
        savedNote={saveStatus === 'saved' ? t('settings.tools.preferencesSaved') : null}
        error={saveStatus === 'error' ? t('settings.tools.saveFailed') : null}
        savingLabel=""
      />
    </>
  );

  // Embedded (onboarding custom wizard) keeps the headerless PanelPage branch.
  if (embedded) {
    return (
      <PanelPage className="z-10" contentClassName="">
        <div className="space-y-4">{body}</div>
      </PanelPage>
    );
  }

  return <SettingsPanel description={t('pages.settings.features.toolsDesc')}>{body}</SettingsPanel>;
};

export default ToolsPanel;
