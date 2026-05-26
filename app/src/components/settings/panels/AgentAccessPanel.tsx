import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type AutonomyLevel,
  isTauri,
  openhumanGetAutonomySettings,
  openhumanUpdateAutonomySettings,
  type TrustedAccess,
  type TrustedRoot,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

// Installs are always *available* but never silent: every `install_tool` call
// is routed through the approval gate, so the user is asked to Approve/Deny each
// install in chat. There is therefore no per-user "disable installs" knob here —
// the consent is captured per-install by the gate, not by a static config flag.
const ALLOW_TOOL_INSTALL = true;

interface PresetOption {
  id: AutonomyLevel;
  title: string;
  description: string;
}

const AgentAccessPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  // Tier presets — built inside the component so titles/descriptions resolve
  // through `t()` (i18n). Order matters: it's the display order.
  const presets: PresetOption[] = [
    {
      id: 'readonly',
      title: t('settings.agentAccess.tier.readonly.title'),
      description: t('settings.agentAccess.tier.readonly.desc'),
    },
    {
      id: 'supervised',
      title: t('settings.agentAccess.tier.supervised.title'),
      description: t('settings.agentAccess.tier.supervised.desc'),
    },
    {
      id: 'full',
      title: t('settings.agentAccess.tier.full.title'),
      description: t('settings.agentAccess.tier.full.desc'),
    },
  ];

  const [level, setLevel] = useState<AutonomyLevel>('supervised');
  const [workspaceOnly, setWorkspaceOnly] = useState(false);
  const [trustedRoots, setTrustedRoots] = useState<TrustedRoot[]>([]);

  const [newRootPath, setNewRootPath] = useState('');
  const [newRootAccess, setNewRootAccess] = useState<TrustedAccess>('read');

  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedNote, setSavedNote] = useState<string | null>(null);
  // Monotonic guard so out-of-order auto-save responses can't clobber UI state
  // with a stale result (last write wins).
  const persistSeqRef = useRef(0);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      if (!isTauri()) {
        setIsLoading(false);
        return;
      }
      try {
        const resp = await openhumanGetAutonomySettings();
        if (cancelled) return;
        setLevel(resp.result.level);
        setWorkspaceOnly(resp.result.workspace_only);
        setTrustedRoots(resp.result.trusted_roots ?? []);
      } catch (e) {
        if (!cancelled)
          setError(e instanceof Error ? e.message : t('settings.agentAccess.loadError'));
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

  // Auto-apply: every change persists immediately (no separate Save button).
  // `allow_tool_install` is fixed; tier, workspace_only and granted folders
  // vary. Pass explicit `next` values (setState is async).
  const persist = async (next: {
    level: AutonomyLevel;
    workspaceOnly: boolean;
    trustedRoots: TrustedRoot[];
  }) => {
    const seq = ++persistSeqRef.current;
    if (!isTauri()) return;
    setError(null);
    setSavedNote(null);
    setIsSaving(true);
    try {
      await openhumanUpdateAutonomySettings({
        level: next.level,
        workspace_only: next.workspaceOnly,
        trusted_roots: next.trustedRoots,
        allow_tool_install: ALLOW_TOOL_INSTALL,
      });
      // Only the most recent persist may write UI state back.
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
    void persist({ level: next, workspaceOnly, trustedRoots });
  };

  const toggleWorkspaceOnly = (next: boolean) => {
    setWorkspaceOnly(next);
    void persist({ level, workspaceOnly: next, trustedRoots });
  };

  const addRoot = () => {
    const path = newRootPath.trim();
    if (!path) return;
    if (trustedRoots.some(r => r.path === path)) {
      setNewRootPath('');
      return;
    }
    const nextRoots = [...trustedRoots, { path, access: newRootAccess }];
    setTrustedRoots(nextRoots);
    setNewRootPath('');
    setNewRootAccess('read');
    void persist({ level, workspaceOnly, trustedRoots: nextRoots });
  };

  const removeRoot = (path: string) => {
    const nextRoots = trustedRoots.filter(r => r.path !== path);
    setTrustedRoots(nextRoots);
    void persist({ level, workspaceOnly, trustedRoots: nextRoots });
  };

  return (
    <div>
      <SettingsHeader
        title={t('settings.agentAccess.title')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-6">
        {!isTauri() && (
          <p className="text-sm text-coral">{t('settings.agentAccess.desktopOnly')}</p>
        )}

        {isLoading ? (
          <p className="text-sm text-ink-soft">{t('settings.agentAccess.loading')}</p>
        ) : (
          <>
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-ink">
                {t('settings.agentAccess.accessMode')}
              </h2>
              <div className="grid gap-2">
                {presets.map(p => (
                  <button
                    key={p.id}
                    type="button"
                    onClick={() => selectTier(p.id)}
                    className={`text-left rounded-lg border p-3 transition ${
                      level === p.id
                        ? 'border-primary-500 bg-primary-50'
                        : 'border-line hover:border-primary-300'
                    }`}>
                    <div className="flex items-center gap-2">
                      <span
                        className={`inline-block w-3 h-3 rounded-full border ${
                          level === p.id ? 'bg-primary-500 border-primary-500' : 'border-line'
                        }`}
                      />
                      <span className="font-medium text-ink">{p.title}</span>
                      {p.id === 'supervised' && (
                        <span className="text-xs text-ink-soft">
                          {t('settings.agentAccess.defaultTag')}
                        </span>
                      )}
                    </div>
                    <p className="mt-1 text-xs text-ink-soft">{p.description}</p>
                  </button>
                ))}
                {level === 'full' && (
                  <p className="rounded border border-coral/40 bg-coral/5 p-2 text-xs text-coral">
                    {t('settings.agentAccess.fullWarning')}
                  </p>
                )}
              </div>
            </section>

            {/* Workspace confinement — orthogonal to the tier; applies in all modes. */}
            <section className="space-y-1">
              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  className="mt-0.5 cursor-pointer"
                  checked={workspaceOnly}
                  onChange={e => toggleWorkspaceOnly(e.target.checked)}
                />
                <span>
                  <span className="text-sm font-medium text-ink">
                    {t('settings.agentAccess.confine.label')}
                  </span>
                  <span className="block text-xs text-ink-soft">
                    {t('settings.agentAccess.confine.desc')}
                  </span>
                </span>
              </label>
            </section>

            {/* Granted folders (trusted roots) — extra read/write reach. */}
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-ink">
                {t('settings.agentAccess.grantedFolders')}
              </h2>
              <p className="text-xs text-ink-soft">{t('settings.agentAccess.grantedDesc')}</p>
              {trustedRoots.length === 0 ? (
                <p className="text-xs text-ink-soft">{t('settings.agentAccess.noneGranted')}</p>
              ) : (
                <ul className="space-y-1">
                  {trustedRoots.map(r => (
                    <li
                      key={r.path}
                      className="flex items-center justify-between rounded border border-line px-2 py-1">
                      <span className="font-mono text-xs text-ink truncate">{r.path}</span>
                      <span className="flex items-center gap-2">
                        <span className="text-xs text-ink-soft">
                          {r.access === 'readwrite'
                            ? t('settings.agentAccess.readWrite')
                            : t('settings.agentAccess.readOnly')}
                        </span>
                        <button
                          type="button"
                          onClick={() => removeRoot(r.path)}
                          className="text-xs text-coral hover:underline">
                          {t('settings.agentAccess.remove')}
                        </button>
                      </span>
                    </li>
                  ))}
                </ul>
              )}
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={newRootPath}
                  onChange={e => setNewRootPath(e.target.value)}
                  placeholder={t('settings.agentAccess.pathPlaceholder')}
                  aria-label={t('settings.agentAccess.pathPlaceholder')}
                  className="flex-1 rounded border border-line px-2 py-1 text-xs font-mono"
                />
                <select
                  value={newRootAccess}
                  onChange={e => setNewRootAccess(e.target.value as TrustedAccess)}
                  aria-label={t('settings.agentAccess.accessLevelLabel')}
                  className="rounded border border-line px-2 py-1 text-xs">
                  <option value="read">{t('settings.agentAccess.readOnly')}</option>
                  <option value="readwrite">{t('settings.agentAccess.readWrite')}</option>
                </select>
                <button
                  type="button"
                  onClick={addRoot}
                  className="rounded bg-primary-500 px-3 py-1 text-xs text-white hover:bg-primary-600">
                  {t('settings.agentAccess.add')}
                </button>
              </div>
            </section>

            {/* Auto-save status — changes persist on selection; no manual save. */}
            <div className="min-h-[1.25rem] text-sm" aria-live="polite">
              {error ? (
                <span className="text-coral">{error}</span>
              ) : isSaving ? (
                <span className="text-ink-soft">{t('settings.agentAccess.saving')}</span>
              ) : savedNote ? (
                <span className="text-sage">✓ {savedNote}</span>
              ) : (
                <span className="text-ink-soft">{t('settings.agentAccess.changesApply')}</span>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
};

export default AgentAccessPanel;
