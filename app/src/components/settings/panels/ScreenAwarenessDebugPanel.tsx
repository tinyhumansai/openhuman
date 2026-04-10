import { type ComponentProps, useEffect, useRef, useState } from 'react';

import ScreenIntelligenceDebugPanel from '../../../components/intelligence/ScreenIntelligenceDebugPanel';
import { useScreenIntelligenceState } from '../../../features/screen-intelligence/useScreenIntelligenceState';
import { isTauri, openhumanUpdateScreenIntelligenceSettings } from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const DebugSection = ({
  state,
}: {
  state: ComponentProps<typeof ScreenIntelligenceDebugPanel>['state'];
}) => {
  const [isOpen, setIsOpen] = useState(false);

  return (
    <section className="space-y-3">
      <button
        type="button"
        onClick={() => setIsOpen(prev => !prev)}
        className="flex w-full items-center justify-between text-sm font-semibold text-stone-900">
        <span>Debug & Diagnostics</span>
        <span className="text-xs text-stone-400">{isOpen ? 'Collapse' : 'Expand'}</span>
      </button>
      {isOpen && <ScreenIntelligenceDebugPanel state={state} />}
    </section>
  );
};

const ScreenAwarenessDebugPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const {
    status,
    lastError,
    isLoadingVision,
    recentVisionSummaries,
    refreshStatus,
    refreshVision,
    runCaptureTest,
    captureTestResult,
    isCaptureTestRunning,
  } = useScreenIntelligenceState({ loadVision: true, visionLimit: 10, pollMs: 2000 });

  const [baselineFps, setBaselineFps] = useState<string>('1');
  const [useVisionModel, setUseVisionModel] = useState<boolean>(true);
  const [keepScreenshots, setKeepScreenshots] = useState<boolean>(false);
  const [allowlistText, setAllowlistText] = useState('');
  const [denylistText, setDenylistText] = useState('');
  const [isSavingConfig, setIsSavingConfig] = useState(false);
  const [configError, setConfigError] = useState<string | null>(null);

  // CoreStateProvider polls every 2s, producing a new `status` object reference on
  // every tick even when the config is unchanged. Compare the serialized value instead
  // so we only re-sync when the server config has actually changed.
  const lastSyncedConfigSigRef = useRef<string | null>(null);
  useEffect(() => {
    if (!status?.config) {
      return;
    }
    const sig = JSON.stringify(status.config);
    if (lastSyncedConfigSigRef.current === sig) {
      return;
    }
    lastSyncedConfigSigRef.current = sig;
    setBaselineFps(String(status.config.baseline_fps ?? 1));
    setUseVisionModel(status.config.use_vision_model ?? true);
    setKeepScreenshots(status.config.keep_screenshots ?? false);
    setAllowlistText((status.config.allowlist ?? []).join('\n'));
    setDenylistText((status.config.denylist ?? []).join('\n'));
  }, [status?.config]);

  const saveConfig = async () => {
    if (!isTauri()) return;
    setConfigError(null);
    setIsSavingConfig(true);
    try {
      const fps = Number(baselineFps);
      await openhumanUpdateScreenIntelligenceSettings({
        enabled: status?.config.enabled ?? false,
        policy_mode:
          status?.config.policy_mode === 'whitelist_only'
            ? 'whitelist_only'
            : 'all_except_blacklist',
        baseline_fps: Number.isFinite(fps) && fps > 0 ? fps : 1,
        use_vision_model: useVisionModel,
        keep_screenshots: keepScreenshots,
        allowlist: allowlistText
          .split('\n')
          .map(v => v.trim())
          .filter(Boolean),
        denylist: denylistText
          .split('\n')
          .map(v => v.trim())
          .filter(Boolean),
      });
      await refreshStatus();
    } catch (error) {
      setConfigError(error instanceof Error ? error.message : 'Failed to save screen intelligence');
    } finally {
      setIsSavingConfig(false);
    }
  };

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="Screen Awareness Debug"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="max-w-2xl mx-auto w-full p-4 space-y-4">
        {/* Advanced policy settings */}
        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Screen Intelligence Policy</h3>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <span className="text-sm text-stone-700">Baseline FPS</span>
            <input
              type="number"
              min={0.2}
              max={30}
              step={0.1}
              value={baselineFps}
              onChange={event => setBaselineFps(event.target.value)}
              className="w-24 rounded border border-stone-200 bg-white px-2 py-1 text-xs text-stone-700"
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <div>
              <span className="text-sm text-stone-700">Use Vision Model</span>
              <p className="text-xs text-stone-400">
                Send screenshots to a vision LLM for richer context. When off, only OCR text is used
                with a text LLM — faster and no vision model required.
              </p>
            </div>
            <input
              type="checkbox"
              checked={useVisionModel}
              onChange={event => setUseVisionModel(event.target.checked)}
            />
          </label>

          <label className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
            <div>
              <span className="text-sm text-stone-700">Keep Screenshots</span>
              <p className="text-xs text-stone-400">
                Save captured screenshots to the workspace instead of deleting after processing
              </p>
            </div>
            <input
              type="checkbox"
              checked={keepScreenshots}
              onChange={event => setKeepScreenshots(event.target.checked)}
            />
          </label>

          <div className="space-y-1">
            <div className="text-xs text-stone-600">Allowlist (one rule per line)</div>
            <textarea
              value={allowlistText}
              onChange={event => setAllowlistText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700"
            />
          </div>

          <div className="space-y-1">
            <div className="text-xs text-stone-600">Denylist (one rule per line)</div>
            <textarea
              value={denylistText}
              onChange={event => setDenylistText(event.target.value)}
              rows={3}
              className="w-full rounded border border-stone-200 bg-stone-50 p-2 text-xs text-stone-700"
            />
          </div>

          <button
            type="button"
            onClick={() => void saveConfig()}
            disabled={isSavingConfig}
            className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
            {isSavingConfig ? 'Saving…' : 'Save Screen Intelligence Settings'}
          </button>
          {configError && <div className="text-xs text-red-600">{configError}</div>}
        </section>

        {/* Session stats */}
        <section className="space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Session Stats</h3>
          <div className="text-sm text-stone-600 space-y-1">
            <div>Frames (ephemeral): {status?.session.frames_in_memory ?? 0}</div>
            <div>Panic stop: {status?.session.panic_hotkey ?? 'Cmd+Shift+.'}</div>
            <div>Vision: {status?.session.vision_state ?? 'idle'}</div>
            <div>Vision queue: {status?.session.vision_queue_depth ?? 0}</div>
            <div>
              Last vision:{' '}
              {status?.session.last_vision_at_ms
                ? new Date(status.session.last_vision_at_ms).toLocaleTimeString()
                : 'n/a'}
            </div>
          </div>
        </section>

        {/* Vision summaries */}
        <section className="space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold text-stone-900">Vision Summaries</h3>
            <button
              type="button"
              onClick={() => void refreshVision(10)}
              disabled={isLoadingVision}
              className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs text-stone-600 disabled:opacity-50">
              {isLoadingVision ? 'Refreshing…' : 'Refresh'}
            </button>
          </div>

          {recentVisionSummaries.length === 0 ? (
            <div className="text-xs text-stone-500">No summaries yet.</div>
          ) : (
            <div className="space-y-2">
              {recentVisionSummaries.map(summary => (
                <div
                  key={summary.id}
                  className="rounded-xl border border-stone-200 bg-white p-3 text-xs text-stone-200">
                  <div className="text-stone-500">
                    {new Date(summary.captured_at_ms).toLocaleTimeString()} ·{' '}
                    {summary.app_name ?? 'Unknown App'}
                    {summary.window_title ? ` · ${summary.window_title}` : ''}
                  </div>
                  <div className="mt-1 text-stone-800">{summary.actionable_notes}</div>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* Debug & Diagnostics (collapsible) */}
        <DebugSection
          state={{
            status,
            recentVisionSummaries,
            lastError,
            captureTestResult,
            isCaptureTestRunning,
            refreshStatus,
            refreshVision,
            runCaptureTest,
          }}
        />

        {/* Platform unsupported notice */}
        {status !== null && !status.platform_supported && (
          <div className="rounded-xl border border-amber-300 bg-amber-50 p-3 text-sm text-amber-700">
            Screen Intelligence V1 is currently supported on macOS only.
          </div>
        )}

        {/* Error notice */}
        {lastError && (
          <div className="rounded-xl border border-red-300 bg-red-50 p-3 text-sm text-red-600">
            {lastError}
          </div>
        )}
      </div>
    </div>
  );
};

export default ScreenAwarenessDebugPanel;
