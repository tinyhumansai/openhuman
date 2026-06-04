import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  isTauri,
  openhumanGetSandboxSettings,
  openhumanUpdateSandboxSettings,
  type SandboxBackendId,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const BACKEND_OPTIONS: SandboxBackendId[] = [
  'auto',
  'docker',
  'landlock',
  'firejail',
  'bubblewrap',
  'none',
];

const SandboxSettingsPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [isLoading, setIsLoading] = useState(isTauri());
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedNote, setSavedNote] = useState<string | null>(null);

  const [enabled, setEnabled] = useState(true);
  const [backend, setBackend] = useState<SandboxBackendId>('auto');
  const [dockerImage, setDockerImage] = useState('alpine:3.20');
  const [memoryLimitMb, setMemoryLimitMb] = useState('512');
  const [cpuLimit, setCpuLimit] = useState('1.0');
  const [dockerAvailable, setDockerAvailable] = useState(false);
  const [detectedBackend, setDetectedBackend] = useState('');
  const [envPassthrough, setEnvPassthrough] = useState<string[]>([]);

  const persistSeqRef = useRef(0);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      if (!isTauri()) return;
      try {
        const resp = await openhumanGetSandboxSettings();
        if (cancelled) return;
        const s = resp.result;
        setEnabled(s.enabled);
        setBackend(s.backend);
        setDockerImage(s.docker_image);
        setMemoryLimitMb(s.docker_memory_limit_mb != null ? String(s.docker_memory_limit_mb) : '');
        setCpuLimit(s.docker_cpu_limit != null ? String(s.docker_cpu_limit) : '');
        setDockerAvailable(s.docker_available);
        setDetectedBackend(s.detected_backend);
        setEnvPassthrough(s.env_passthrough);
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : t('settings.sandbox.loadError'));
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const persist = async (patch: Parameters<typeof openhumanUpdateSandboxSettings>[0]) => {
    const seq = ++persistSeqRef.current;
    if (!isTauri()) return;
    setError(null);
    setSavedNote(null);
    setIsSaving(true);
    try {
      await openhumanUpdateSandboxSettings(patch);
      if (seq !== persistSeqRef.current) return;
      setSavedNote(t('settings.sandbox.saved'));
    } catch (e) {
      if (seq !== persistSeqRef.current) return;
      setError(e instanceof Error ? e.message : t('settings.sandbox.saveError'));
    } finally {
      if (seq === persistSeqRef.current) setIsSaving(false);
    }
  };

  const handleBackendChange = (next: SandboxBackendId) => {
    setBackend(next);
    void persist({ backend: next });
  };

  const handleEnabledChange = (next: boolean) => {
    setEnabled(next);
    void persist({ enabled: next });
  };

  const handleDockerImageBlur = () => {
    if (dockerImage.trim()) {
      void persist({ docker_image: dockerImage.trim() });
    }
  };

  const handleMemoryBlur = () => {
    if (memoryLimitMb.trim() === '') {
      void persist({ docker_memory_limit_mb: null });
      return;
    }
    const parsed = parseInt(memoryLimitMb, 10);
    if (!isNaN(parsed) && parsed > 0) {
      void persist({ docker_memory_limit_mb: parsed });
    }
  };

  const handleCpuBlur = () => {
    if (cpuLimit.trim() === '') {
      void persist({ docker_cpu_limit: null });
      return;
    }
    const parsed = parseFloat(cpuLimit);
    if (!isNaN(parsed) && parsed > 0) {
      void persist({ docker_cpu_limit: parsed });
    }
  };

  if (!isTauri()) {
    return (
      <div className="mx-auto max-w-2xl px-4 py-8">
        <SettingsHeader
          title={t('settings.sandbox.title')}
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
        <p className="text-sm text-stone-500 dark:text-stone-400">
          {t('settings.sandbox.desktopOnly')}
        </p>
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="mx-auto max-w-2xl px-4 py-8">
        <SettingsHeader
          title={t('settings.sandbox.title')}
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
        <p className="text-sm text-stone-500 dark:text-stone-400">
          {t('settings.sandbox.loading')}
        </p>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-2xl px-4 py-8">
      <SettingsHeader
        title={t('settings.sandbox.title')}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      {error && (
        <div
          className="mb-4 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300"
          role="alert">
          {error}
        </div>
      )}

      {savedNote && (
        <p className="mb-4 text-sm text-green-600 dark:text-green-400" aria-live="polite">
          {savedNote}
        </p>
      )}

      {/* Status */}
      <section className="mb-6">
        <h2 className="mb-2 text-sm font-semibold text-stone-700 dark:text-stone-200">
          {t('settings.sandbox.status')}
        </h2>
        <div className="space-y-2 rounded-lg border border-stone-200 bg-stone-50 p-4 dark:border-stone-700 dark:bg-stone-800">
          <div className="flex items-center justify-between">
            <span className="text-sm text-stone-600 dark:text-stone-300">
              {t('settings.sandbox.dockerStatus')}
            </span>
            <span
              className={`rounded-full px-2 py-0.5 text-xs font-medium ${
                dockerAvailable
                  ? 'bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300'
                  : 'bg-stone-200 text-stone-600 dark:bg-stone-700 dark:text-stone-400'
              }`}>
              {dockerAvailable
                ? t('settings.sandbox.available')
                : t('settings.sandbox.unavailable')}
            </span>
          </div>
          {detectedBackend && (
            <div className="flex items-center justify-between">
              <span className="text-sm text-stone-600 dark:text-stone-300">
                {t('settings.sandbox.detectedBackend')}
              </span>
              <span className="text-sm font-mono text-stone-700 dark:text-stone-200">
                {detectedBackend}
              </span>
            </div>
          )}
        </div>
      </section>

      {/* Enabled toggle */}
      <section className="mb-6">
        <label className="flex items-center gap-3">
          <input
            type="checkbox"
            checked={enabled}
            onChange={e => handleEnabledChange(e.target.checked)}
            className="h-4 w-4 rounded border-stone-300 dark:border-stone-600"
            aria-label={t('settings.sandbox.enableLabel')}
          />
          <div>
            <span className="text-sm font-medium text-stone-700 dark:text-stone-200">
              {t('settings.sandbox.enableLabel')}
            </span>
            <p className="text-xs text-stone-500 dark:text-stone-400">
              {t('settings.sandbox.enableDesc')}
            </p>
          </div>
        </label>
      </section>

      {/* Backend selection */}
      <section className="mb-6">
        <h2 className="mb-2 text-sm font-semibold text-stone-700 dark:text-stone-200">
          {t('settings.sandbox.backendLabel')}
        </h2>
        <p className="mb-2 text-xs text-stone-500 dark:text-stone-400">
          {t('settings.sandbox.backendDesc')}
        </p>
        <select
          value={backend}
          onChange={e => handleBackendChange(e.target.value as SandboxBackendId)}
          className="w-full rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-700 dark:border-stone-600 dark:bg-stone-800 dark:text-stone-200"
          aria-label={t('settings.sandbox.backendLabel')}>
          {BACKEND_OPTIONS.map(opt => (
            <option key={opt} value={opt}>
              {t(`settings.sandbox.backend.${opt}`)}
            </option>
          ))}
        </select>
      </section>

      {/* Docker settings */}
      <section className="mb-6">
        <h2 className="mb-2 text-sm font-semibold text-stone-700 dark:text-stone-200">
          {t('settings.sandbox.dockerSettings')}
        </h2>
        <div className="space-y-4 rounded-lg border border-stone-200 p-4 dark:border-stone-700">
          {/* Docker image */}
          <div>
            <label className="mb-1 block text-xs font-medium text-stone-600 dark:text-stone-300">
              {t('settings.sandbox.dockerImage')}
            </label>
            <input
              type="text"
              value={dockerImage}
              onChange={e => setDockerImage(e.target.value)}
              onBlur={handleDockerImageBlur}
              onKeyDown={e => e.key === 'Enter' && handleDockerImageBlur()}
              className="w-full rounded-lg border border-stone-300 bg-white px-3 py-2 font-mono text-sm text-stone-700 dark:border-stone-600 dark:bg-stone-800 dark:text-stone-200"
              aria-label={t('settings.sandbox.dockerImage')}
              placeholder={t('settings.sandbox.dockerImagePlaceholder')}
            />
          </div>

          {/* Memory limit */}
          <div>
            <label className="mb-1 block text-xs font-medium text-stone-600 dark:text-stone-300">
              {t('settings.sandbox.memoryLimit')}
            </label>
            <div className="flex items-center gap-2">
              <input
                type="number"
                value={memoryLimitMb}
                onChange={e => setMemoryLimitMb(e.target.value)}
                onBlur={handleMemoryBlur}
                onKeyDown={e => e.key === 'Enter' && handleMemoryBlur()}
                className="w-32 rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-700 dark:border-stone-600 dark:bg-stone-800 dark:text-stone-200"
                aria-label={t('settings.sandbox.memoryLimit')}
                min={64}
              />
              <span className="text-xs text-stone-500 dark:text-stone-400">
                {t('settings.sandbox.memoryUnit')}
              </span>
            </div>
          </div>

          {/* CPU limit */}
          <div>
            <label className="mb-1 block text-xs font-medium text-stone-600 dark:text-stone-300">
              {t('settings.sandbox.cpuLimit')}
            </label>
            <div className="flex items-center gap-2">
              <input
                type="number"
                value={cpuLimit}
                onChange={e => setCpuLimit(e.target.value)}
                onBlur={handleCpuBlur}
                onKeyDown={e => e.key === 'Enter' && handleCpuBlur()}
                className="w-32 rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-700 dark:border-stone-600 dark:bg-stone-800 dark:text-stone-200"
                aria-label={t('settings.sandbox.cpuLimit')}
                min={0.1}
                step={0.1}
              />
              <span className="text-xs text-stone-500 dark:text-stone-400">
                {t('settings.sandbox.cpuUnit')}
              </span>
            </div>
          </div>
        </div>
      </section>

      {/* Environment passthrough */}
      <section className="mb-6">
        <h2 className="mb-2 text-sm font-semibold text-stone-700 dark:text-stone-200">
          {t('settings.sandbox.envPassthrough')}
        </h2>
        <p className="mb-2 text-xs text-stone-500 dark:text-stone-400">
          {t('settings.sandbox.envPassthroughDesc')}
        </p>
        <div className="rounded-lg border border-stone-200 bg-stone-50 p-3 dark:border-stone-700 dark:bg-stone-800">
          {envPassthrough.length > 0 ? (
            <div className="flex flex-wrap gap-2">
              {envPassthrough.map(v => (
                <span
                  key={v}
                  className="rounded-md bg-stone-200 px-2 py-0.5 font-mono text-xs text-stone-700 dark:bg-stone-700 dark:text-stone-300">
                  {v}
                </span>
              ))}
            </div>
          ) : (
            <p className="text-xs text-stone-400 dark:text-stone-500">
              {t('settings.sandbox.noEnvVars')}
            </p>
          )}
        </div>
      </section>

      {/* Saving indicator */}
      {isSaving && (
        <p className="text-xs text-stone-500 dark:text-stone-400" aria-live="polite">
          {t('settings.sandbox.saving')}
        </p>
      )}
    </div>
  );
};

export default SandboxSettingsPanel;
