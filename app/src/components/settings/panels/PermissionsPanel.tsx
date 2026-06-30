import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type AgentPaths,
  type AutonomyLevel,
  isTauri,
  openhumanGetAgentPaths,
  openhumanGetAutonomySettings,
  openhumanUpdateAgentPaths,
  openhumanUpdateAutonomySettings,
} from '../../../utils/tauriCommands';
import Button from '../../ui/Button';
import { SettingsStatusLine, SettingsTextField } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

// Installs are always *available* but never silent: every `install_tool` call
// is routed through the approval gate, so the user is asked to Approve/Deny
// each install in chat. There is no per-user "disable installs" knob here —
// the consent is captured per-install by the gate, not by a static config flag.
const ALLOW_TOOL_INSTALL = true;

interface PresetOption {
  id: AutonomyLevel;
  title: string;
  description: string;
}

const PermissionsPanel = () => {
  const { t } = useT();

  // Tier presets — built inside the component so titles/descriptions resolve
  // through `t()` (i18n). Order matters: it's the display order.
  const presets: PresetOption[] = [
    {
      id: 'readonly',
      title: t('settings.permissions.preset.readonly.title'),
      description: t('settings.permissions.preset.readonly.desc'),
    },
    {
      id: 'supervised',
      title: t('settings.permissions.preset.supervised.title'),
      description: t('settings.permissions.preset.supervised.desc'),
    },
    {
      id: 'full',
      title: t('settings.permissions.preset.full.title'),
      description: t('settings.permissions.preset.full.desc'),
    },
  ];

  const [level, setLevel] = useState<AutonomyLevel>('supervised');
  // We need to carry workspace_only and trusted_roots when saving tier changes
  // so we don't overwrite them with defaults. Load them but don't expose UI for
  // them (they live in the advanced panel).
  const [workspaceOnly, setWorkspaceOnly] = useState(false);
  const [requireTaskPlanApproval, setRequireTaskPlanApproval] = useState(true);
  const [trustedRoots, setTrustedRoots] = useState<
    Array<{ path: string; access: 'read' | 'readwrite' }>
  >([]);

  const [agentPaths, setAgentPaths] = useState<AgentPaths | null>(null);
  const [actionDirEditing, setActionDirEditing] = useState(false);
  const [actionDirInput, setActionDirInput] = useState('');
  const [actionDirError, setActionDirError] = useState<string | null>(null);
  const [actionDirSaved, setActionDirSaved] = useState<string | null>(null);
  const [actionDirSaving, setActionDirSaving] = useState(false);

  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedNote, setSavedNote] = useState<string | null>(null);

  // Monotonic guards so out-of-order async responses don't clobber UI state.
  const persistSeqRef = useRef(0);
  const dirSeqRef = useRef(0);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      if (!isTauri()) {
        setIsLoading(false);
        return;
      }
      try {
        const autonomyResp = await openhumanGetAutonomySettings();
        if (cancelled) return;
        setLevel(autonomyResp.result.level);
        setWorkspaceOnly(autonomyResp.result.workspace_only);
        setRequireTaskPlanApproval(autonomyResp.result.require_task_plan_approval ?? true);
        setTrustedRoots(autonomyResp.result.trusted_roots ?? []);
      } catch (e) {
        if (!cancelled)
          setError(e instanceof Error ? e.message : t('settings.agentAccess.loadError'));
      }
      try {
        const pathsResp = await openhumanGetAgentPaths();
        if (cancelled) return;
        setAgentPaths(pathsResp.result);
        setActionDirInput(pathsResp.result.action_dir);
      } catch {
        // Non-fatal: folder section falls back to documented defaults.
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

  // Persist tier change. Carries the other autonomy fields through unchanged
  // so we don't accidentally clobber what the advanced panel may have set.
  const persistTier = async (nextLevel: AutonomyLevel) => {
    const seq = ++persistSeqRef.current;
    if (!isTauri()) return;
    setError(null);
    setSavedNote(null);
    setIsSaving(true);
    try {
      await openhumanUpdateAutonomySettings({
        level: nextLevel,
        workspace_only: workspaceOnly,
        trusted_roots: trustedRoots,
        allow_tool_install: ALLOW_TOOL_INSTALL,
        require_task_plan_approval: requireTaskPlanApproval,
      });
      if (persistSeqRef.current === seq) {
        setSavedNote(t('settings.agentAccess.saved'));
      }
    } catch (e) {
      if (persistSeqRef.current === seq) {
        setError(e instanceof Error ? e.message : t('settings.agentAccess.saveError'));
      }
    } finally {
      if (persistSeqRef.current === seq) {
        setIsSaving(false);
      }
    }
  };

  const selectTier = (next: AutonomyLevel) => {
    setLevel(next);
    void persistTier(next);
  };

  // True when the env var pins action_dir — the edit button must be hidden.
  const actionDirEnvLocked = agentPaths?.action_dir_source === 'env';

  const startEditActionDir = () => {
    setActionDirInput(agentPaths?.action_dir ?? '');
    setActionDirError(null);
    setActionDirSaved(null);
    setActionDirEditing(true);
  };

  const cancelEditActionDir = () => {
    setActionDirEditing(false);
    setActionDirError(null);
    setActionDirInput('');
  };

  const saveActionDir = async () => {
    if (!isTauri()) return;
    const seq = ++dirSeqRef.current;
    setActionDirSaving(true);
    setActionDirError(null);
    setActionDirSaved(null);
    try {
      const resp = await openhumanUpdateAgentPaths({ action_dir: actionDirInput.trim() });
      if (dirSeqRef.current === seq) {
        setAgentPaths(resp.result);
        setActionDirEditing(false);
        setActionDirSaved(t('settings.agentAccess.actionDir.saved'));
      }
    } catch (e) {
      if (dirSeqRef.current === seq) {
        setActionDirError(e instanceof Error ? e.message : t('settings.agentAccess.saveError'));
      }
    } finally {
      if (dirSeqRef.current === seq) {
        setActionDirSaving(false);
      }
    }
  };

  return (
    <SettingsPanel>
      <div className="space-y-5">
        {!isTauri() && (
          <p className="text-sm text-coral-600 dark:text-coral-300">
            {t('settings.agentAccess.desktopOnly')}
          </p>
        )}

        {isLoading ? (
          <p className="text-sm text-content-muted">{t('settings.agentAccess.loading')}</p>
        ) : (
          <>
            {/* Access mode presets — intentional bespoke card UI; kept as-is. */}
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-content">
                {t('settings.permissions.accessMode')}
              </h2>
              <p className="text-xs text-content-muted">
                {t('settings.permissions.accessModeDesc')}
              </p>
              <div className="grid gap-2">
                {presets.map(p => (
                  <Button
                    key={p.id}
                    type="button"
                    variant="tertiary"
                    onClick={() => selectTier(p.id)}
                    data-testid={`permissions-preset-${p.id}`}
                    className={`!inline-block h-auto w-full !justify-start text-left rounded-lg border p-3 transition ${
                      level === p.id
                        ? 'border-primary-500 bg-primary-50 dark:bg-primary-500/10'
                        : 'border-line hover:border-primary-300 dark:hover:border-primary-500'
                    }`}>
                    <div className="flex items-center gap-2">
                      <span
                        className={`inline-block w-3 h-3 rounded-full border ${
                          level === p.id
                            ? 'bg-primary-500 border-primary-500'
                            : 'border-line-strong'
                        }`}
                      />
                      <span className="font-medium text-content">{p.title}</span>
                      {p.id === 'supervised' && (
                        <span className="text-xs text-content-muted">
                          {t('settings.agentAccess.defaultTag')}
                        </span>
                      )}
                    </div>
                    <p className="mt-1 text-xs text-content-muted">{p.description}</p>
                  </Button>
                ))}
                {level === 'full' && (
                  <p className="rounded border border-coral/40 bg-coral/5 dark:bg-coral/10 p-2 text-xs text-coral-600 dark:text-coral-300">
                    {t('settings.agentAccess.fullWarning')}
                  </p>
                )}
              </div>
            </section>

            {/* Folders the assistant can use */}
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-content">
                {t('settings.permissions.folders')}
              </h2>
              <p className="text-xs text-content-muted">{t('settings.permissions.foldersDesc')}</p>
              <div className="rounded-lg border border-line px-3 py-2">
                <div className="flex items-center gap-2">
                  <span className="inline-block w-2 h-2 rounded-full bg-sage-500" />
                  <span className="text-xs font-medium text-content">
                    {t('settings.agentAccess.actionSandbox')}
                  </span>
                  <span className="text-xs text-sage-600 dark:text-sage-400">
                    {t('settings.agentAccess.readWriteAccess')}
                  </span>
                </div>
                {actionDirEditing ? (
                  <div className="mt-1 space-y-1">
                    <div className="flex items-center gap-2">
                      <SettingsTextField
                        mono
                        className="flex-1"
                        inputSize="sm"
                        value={actionDirInput}
                        onChange={e => setActionDirInput(e.target.value)}
                        placeholder={t('settings.agentAccess.actionDir.placeholder')}
                        disabled={actionDirSaving}
                        data-testid="permissions-action-dir-input"
                      />
                      <Button
                        type="button"
                        variant="primary"
                        size="xs"
                        onClick={() => void saveActionDir()}
                        disabled={actionDirSaving}
                        data-testid="permissions-action-dir-save">
                        {t('settings.agentAccess.actionDir.save')}
                      </Button>
                      <Button
                        type="button"
                        variant="secondary"
                        size="xs"
                        onClick={cancelEditActionDir}
                        disabled={actionDirSaving}
                        data-testid="permissions-action-dir-cancel">
                        {t('settings.agentAccess.actionDir.cancel')}
                      </Button>
                    </div>
                    {actionDirError && (
                      <p
                        className="text-xs text-coral-600 dark:text-coral-400"
                        data-testid="permissions-action-dir-error">
                        {actionDirError}
                      </p>
                    )}
                  </div>
                ) : (
                  <div className="mt-0.5 flex items-center gap-2">
                    <p
                      className="text-xs text-content-muted font-mono"
                      data-testid="permissions-action-dir">
                      {agentPaths?.action_dir ?? '~/OpenHuman/projects'}
                    </p>
                    {!actionDirEnvLocked && (
                      <Button
                        variant="tertiary"
                        size="xs"
                        className="text-ocean hover:underline"
                        onClick={startEditActionDir}
                        data-testid="permissions-action-dir-edit">
                        {t('settings.agentAccess.actionDir.edit')}
                      </Button>
                    )}
                  </div>
                )}
                {actionDirEnvLocked && (
                  <p
                    className="text-xs text-amber-600 dark:text-amber-400"
                    data-testid="permissions-action-dir-env-locked">
                    {t('settings.agentAccess.actionDir.envLocked')}
                  </p>
                )}
                {actionDirSaved && !actionDirEditing && (
                  <p className="text-xs text-sage-600 dark:text-sage-400">{actionDirSaved}</p>
                )}
                <p className="text-xs text-content-muted dark:text-content-faint mt-0.5">
                  {t('settings.agentAccess.actionSandboxDesc')}
                </p>
              </div>
            </section>

            {/* Auto-save status */}
            <SettingsStatusLine
              saving={isSaving}
              savedNote={savedNote}
              error={error}
              savingLabel={t('settings.agentAccess.saving')}
            />
          </>
        )}
      </div>
    </SettingsPanel>
  );
};

export default PermissionsPanel;
