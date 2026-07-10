import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  isTauri,
  openhumanGetSandboxSettings,
  openhumanUpdateSandboxSettings,
  type SandboxBackendId,
} from '../../../utils/tauriCommands';
import {
  SettingsBadge,
  SettingsEmptyState,
  SettingsRow,
  SettingsSection,
  SettingsSelect,
  SettingsStatusLine,
  SettingsSwitch,
  SettingsTextField,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

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
      <SettingsPanel description={t('settings.sandbox.menuDesc')}>
        <p className="text-sm text-content-muted">{t('settings.sandbox.desktopOnly')}</p>
      </SettingsPanel>
    );
  }

  if (isLoading) {
    return (
      <SettingsPanel description={t('settings.sandbox.menuDesc')}>
        <p className="text-sm text-content-muted">{t('settings.sandbox.loading')}</p>
      </SettingsPanel>
    );
  }

  return (
    <SettingsPanel description={t('settings.sandbox.menuDesc')}>
      <>
        {/* Status section */}
        <SettingsSection title={t('settings.sandbox.status')}>
          <SettingsRow
            label={t('settings.sandbox.dockerStatus')}
            control={
              <SettingsBadge variant={dockerAvailable ? 'success' : 'neutral'}>
                {dockerAvailable
                  ? t('settings.sandbox.available')
                  : t('settings.sandbox.unavailable')}
              </SettingsBadge>
            }
          />
          {detectedBackend && (
            <SettingsRow
              label={t('settings.sandbox.detectedBackend')}
              control={<span className="text-sm font-mono text-content">{detectedBackend}</span>}
            />
          )}
        </SettingsSection>

        {/* Enabled toggle */}
        <SettingsSection>
          <SettingsRow
            htmlFor="switch-sandbox-enabled"
            label={t('settings.sandbox.enableLabel')}
            description={t('settings.sandbox.enableDesc')}
            control={
              <SettingsSwitch
                id="switch-sandbox-enabled"
                checked={enabled}
                onCheckedChange={handleEnabledChange}
                aria-label={t('settings.sandbox.enableLabel')}
              />
            }
          />
        </SettingsSection>

        {/* Backend selection */}
        <SettingsSection
          title={t('settings.sandbox.backendLabel')}
          description={t('settings.sandbox.backendDesc')}>
          <SettingsRow
            stacked
            control={
              <SettingsSelect
                value={backend}
                onChange={e => handleBackendChange(e.target.value as SandboxBackendId)}
                aria-label={t('settings.sandbox.backendLabel')}>
                {BACKEND_OPTIONS.map(opt => (
                  <option key={opt} value={opt}>
                    {t(`settings.sandbox.backend.${opt}`)}
                  </option>
                ))}
              </SettingsSelect>
            }
          />
        </SettingsSection>

        {/* Docker settings */}
        <SettingsSection title={t('settings.sandbox.dockerSettings')}>
          {/* Docker image */}
          <SettingsRow
            htmlFor="sandbox-docker-image"
            label={t('settings.sandbox.dockerImage')}
            stacked
            control={
              <SettingsTextField
                id="sandbox-docker-image"
                mono
                value={dockerImage}
                onChange={e => setDockerImage(e.target.value)}
                onBlur={handleDockerImageBlur}
                onKeyDown={e => e.key === 'Enter' && handleDockerImageBlur()}
                aria-label={t('settings.sandbox.dockerImage')}
                placeholder={t('settings.sandbox.dockerImagePlaceholder')}
              />
            }
          />

          {/* Memory limit */}
          <SettingsRow
            htmlFor="sandbox-memory-limit"
            label={t('settings.sandbox.memoryLimit')}
            stacked
            control={
              <div className="flex items-center gap-2">
                <SettingsTextField
                  id="sandbox-memory-limit"
                  type="number"
                  className="w-32"
                  inputSize="sm"
                  value={memoryLimitMb}
                  onChange={e => setMemoryLimitMb(e.target.value)}
                  onBlur={handleMemoryBlur}
                  onKeyDown={e => e.key === 'Enter' && handleMemoryBlur()}
                  aria-label={t('settings.sandbox.memoryLimit')}
                  min={64}
                />
                <span className="text-xs text-content-muted">
                  {t('settings.sandbox.memoryUnit')}
                </span>
              </div>
            }
          />

          {/* CPU limit */}
          <SettingsRow
            htmlFor="sandbox-cpu-limit"
            label={t('settings.sandbox.cpuLimit')}
            stacked
            control={
              <div className="flex items-center gap-2">
                <SettingsTextField
                  id="sandbox-cpu-limit"
                  type="number"
                  className="w-32"
                  inputSize="sm"
                  value={cpuLimit}
                  onChange={e => setCpuLimit(e.target.value)}
                  onBlur={handleCpuBlur}
                  onKeyDown={e => e.key === 'Enter' && handleCpuBlur()}
                  aria-label={t('settings.sandbox.cpuLimit')}
                  min={0.1}
                  step={0.1}
                />
                <span className="text-xs text-content-muted">{t('settings.sandbox.cpuUnit')}</span>
              </div>
            }
          />
        </SettingsSection>

        {/* Environment passthrough */}
        <SettingsSection
          title={t('settings.sandbox.envPassthrough')}
          description={t('settings.sandbox.envPassthroughDesc')}>
          {envPassthrough.length > 0 ? (
            <div className="px-4 py-3 flex flex-wrap gap-2">
              {envPassthrough.map(v => (
                <SettingsBadge key={v} variant="neutral">
                  <span className="font-mono">{v}</span>
                </SettingsBadge>
              ))}
            </div>
          ) : (
            <SettingsEmptyState label={t('settings.sandbox.noEnvVars')} />
          )}
        </SettingsSection>

        {/* Status line */}
        <SettingsStatusLine
          saving={isSaving}
          savedNote={savedNote}
          error={error}
          savingLabel={t('settings.sandbox.saving')}
        />
      </>
    </SettingsPanel>
  );
};

export default SandboxSettingsPanel;
