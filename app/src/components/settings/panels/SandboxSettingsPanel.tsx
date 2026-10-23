import { Box, Container, type LucideIcon, ShieldOff, Sparkles, SquareTerminal } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  openhumanGetSandboxSettings,
  openhumanUpdateSandboxSettings,
  type SandboxBackendId,
} from '../../../utils/tauriCommands';
import {
  Badge,
  Card,
  CenteredLoadingState,
  EmptyState,
  Field,
  InputGroupAddon,
  InputGroupInput,
  InputGroupRoot,
  RadioGroupItem,
  RadioGroupRoot,
  StatusLine,
  Switch,
  TextField,
} from '../../ui';
import SettingsPanel from '../layout/SettingsPanel';

interface BackendOption {
  id: SandboxBackendId;
  icon: LucideIcon;
  /** Linux-only kernel/userland jails; shown with a "Linux" tag. */
  linuxOnly?: boolean;
}

const BACKEND_OPTIONS: BackendOption[] = [
  { id: 'auto', icon: Sparkles },
  { id: 'docker', icon: Container },
  { id: 'landlock', icon: Box, linuxOnly: true },
  { id: 'none', icon: ShieldOff },
];

/**
 * Settings → Security → Sandbox execution. One master switch in the page
 * header card, then the isolation backend as picker tiles, Docker limits (only
 * when Docker can be used), and the read-only env passthrough list. Every
 * control saves on change / blur; the status line under the cards reports it.
 */
const SandboxSettingsPanel = () => {
  const { t } = useT();

  const [isLoading, setIsLoading] = useState(true);
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

  if (isLoading) {
    return (
      <SettingsPanel description={t('settings.sandbox.menuDesc')}>
        <CenteredLoadingState label={t('settings.sandbox.loading')} />
      </SettingsPanel>
    );
  }

  // Docker limits matter when Docker is (or may be) the backend in use.
  const showDocker = enabled && (backend === 'docker' || (backend === 'auto' && dockerAvailable));

  return (
    <SettingsPanel description={t('settings.sandbox.menuDesc')}>
      {/* ── Master switch + what this machine can do ─────────────────── */}
      <Card data-testid="sandbox-status">
        <div className="flex items-center gap-3 p-4">
          <span
            className={cn(
              'flex h-10 w-10 shrink-0 items-center justify-center rounded-lg',
              enabled
                ? 'bg-primary-500 text-content-inverted'
                : 'bg-surface-muted text-content-secondary'
            )}>
            <SquareTerminal className="h-5 w-5" aria-hidden />
          </span>
          <div className="min-w-0 flex-1">
            <label
              htmlFor="switch-sandbox-enabled"
              className="block text-sm font-semibold text-content">
              {t('settings.sandbox.enableLabel')}
            </label>
            <p className="mt-0.5 text-xs text-content-muted">{t('settings.sandbox.enableDesc')}</p>
          </div>
          <Switch
            id="switch-sandbox-enabled"
            checked={enabled}
            onCheckedChange={handleEnabledChange}
            aria-label={t('settings.sandbox.enableLabel')}
          />
        </div>
        <div className="flex flex-wrap items-center gap-x-5 gap-y-2 px-4 py-3 text-xs">
          <span className="flex items-center gap-2 text-content-muted">
            {t('settings.sandbox.dockerStatus')}
            <Badge
              variant={dockerAvailable ? 'success' : 'neutral'}
              data-testid="sandbox-docker-status">
              {dockerAvailable
                ? t('settings.sandbox.available')
                : t('settings.sandbox.unavailable')}
            </Badge>
          </span>
          {detectedBackend && (
            <span className="flex items-center gap-2 text-content-muted">
              {t('settings.sandbox.detectedBackend')}
              <Badge variant="neutral" className="font-mono">
                {detectedBackend}
              </Badge>
            </span>
          )}
        </div>
      </Card>

      {/* ── Isolation backend ─────────────────────────────────────────── */}
      <Card
        title={t('settings.sandbox.backendLabel')}
        description={t('settings.sandbox.backendDesc')}>
        <RadioGroupRoot
          value={backend}
          onValueChange={next => handleBackendChange(next as SandboxBackendId)}
          aria-label={t('settings.sandbox.backendLabel')}
          disabled={!enabled}
          className={cn(
            'grid gap-2 p-4 sm:grid-cols-2 xl:grid-cols-3',
            !enabled && 'pointer-events-none opacity-50'
          )}
          data-testid="sandbox-backend-options">
          {BACKEND_OPTIONS.map(({ id, icon: Icon, linuxOnly }) => {
            const selected = backend === id;
            const inputId = `sandbox-backend-${id}`;
            return (
              <label
                key={id}
                htmlFor={inputId}
                className={cn(
                  'flex cursor-pointer items-center gap-3 rounded-xl border px-3.5 py-3 transition-colors',
                  selected
                    ? 'border-primary-500 bg-primary-50 ring-1 ring-primary-500 dark:bg-primary-500/10'
                    : 'border-line bg-surface hover:border-line-strong hover:bg-surface-hover'
                )}>
                <span
                  className={cn(
                    'flex h-9 w-9 shrink-0 items-center justify-center rounded-lg',
                    selected
                      ? 'bg-primary-500 text-content-inverted'
                      : 'bg-surface-muted text-content-secondary'
                  )}>
                  <Icon className="h-4.5 w-4.5" aria-hidden />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5 text-sm font-semibold text-content">
                    {t(`settings.sandbox.backendName.${id}`)}
                    {linuxOnly && <Badge variant="neutral">{t('settings.sandbox.linuxTag')}</Badge>}
                  </span>
                  <span className="mt-0.5 block text-xs text-content-muted">
                    {t(`settings.sandbox.backendHint.${id}`)}
                  </span>
                </span>
                <RadioGroupItem
                  id={inputId}
                  value={id}
                  data-testid={`sandbox-backend-option-${id}`}
                  className="shrink-0"
                />
              </label>
            );
          })}
        </RadioGroupRoot>
      </Card>

      {/* ── Docker limits ─────────────────────────────────────────────── */}
      {showDocker && (
        <Card title={t('settings.sandbox.dockerSettings')} data-testid="sandbox-docker-settings">
          <Field
            htmlFor="sandbox-docker-image"
            label={t('settings.sandbox.dockerImage')}
            description={t('settings.sandbox.dockerImageDesc')}
            control={
              <TextField
                id="sandbox-docker-image"
                mono
                inputSize="sm"
                className="w-56"
                value={dockerImage}
                onChange={e => setDockerImage(e.target.value)}
                onBlur={handleDockerImageBlur}
                onKeyDown={e => e.key === 'Enter' && handleDockerImageBlur()}
                placeholder={t('settings.sandbox.dockerImagePlaceholder')}
              />
            }
          />
          <Field
            htmlFor="sandbox-memory-limit"
            label={t('settings.sandbox.memoryLimit')}
            description={t('settings.sandbox.limitBlankHint')}
            control={
              <InputGroupRoot size="sm" className="w-40">
                <InputGroupInput
                  id="sandbox-memory-limit"
                  type="number"
                  value={memoryLimitMb}
                  onChange={e => setMemoryLimitMb(e.target.value)}
                  onBlur={handleMemoryBlur}
                  onKeyDown={e => e.key === 'Enter' && handleMemoryBlur()}
                  min={64}
                />
                <InputGroupAddon>{t('settings.sandbox.memoryUnit')}</InputGroupAddon>
              </InputGroupRoot>
            }
          />
          <Field
            htmlFor="sandbox-cpu-limit"
            label={t('settings.sandbox.cpuLimit')}
            description={t('settings.sandbox.limitBlankHint')}
            control={
              <InputGroupRoot size="sm" className="w-40">
                <InputGroupInput
                  id="sandbox-cpu-limit"
                  type="number"
                  value={cpuLimit}
                  onChange={e => setCpuLimit(e.target.value)}
                  onBlur={handleCpuBlur}
                  onKeyDown={e => e.key === 'Enter' && handleCpuBlur()}
                  min={0.1}
                  step={0.1}
                />
                <InputGroupAddon>{t('settings.sandbox.cpuUnit')}</InputGroupAddon>
              </InputGroupRoot>
            }
          />
        </Card>
      )}

      {/* ── Environment passthrough (read-only) ──────────────────────── */}
      <Card
        title={t('settings.sandbox.envPassthrough')}
        description={t('settings.sandbox.envPassthroughDesc')}>
        {envPassthrough.length > 0 ? (
          <div className="flex flex-wrap gap-2 p-4">
            {envPassthrough.map(v => (
              <Badge key={v} variant="neutral" className="font-mono">
                {v}
              </Badge>
            ))}
          </div>
        ) : (
          <div className="p-4">
            <EmptyState label={t('settings.sandbox.noEnvVars')} />
          </div>
        )}
      </Card>

      <StatusLine
        saving={isSaving}
        savedNote={savedNote}
        error={error}
        savingLabel={t('settings.sandbox.saving')}
      />
    </SettingsPanel>
  );
};

export default SandboxSettingsPanel;
