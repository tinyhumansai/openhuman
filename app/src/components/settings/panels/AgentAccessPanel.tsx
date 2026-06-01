import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type AutonomyLevel,
  isTauri,
  openhumanGetAgentSettings,
  openhumanGetAutonomySettings,
  openhumanUpdateAgentSettings,
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
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();

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
  const [requireTaskPlanApproval, setRequireTaskPlanApproval] = useState(true);
  const [trustedRoots, setTrustedRoots] = useState<TrustedRoot[]>([]);
  // "Always allow" allowlist — populated by the in-chat "Always allow" button;
  // shown here read-only with a Remove action (the re-protect path).
  const [autoApprove, setAutoApprove] = useState<string[]>([]);

  const [newRootPath, setNewRootPath] = useState('');
  const [newRootAccess, setNewRootAccess] = useState<TrustedAccess>('read');

  // Action timeout (the tool/action wall-clock limit, issue #3100). Held as the
  // raw input string so the field can be edited freely; validated on save.
  const [timeoutInput, setTimeoutInput] = useState('');
  const [timeoutEnvOverride, setTimeoutEnvOverride] = useState(false);
  const [timeoutMin, setTimeoutMin] = useState(1);
  const [timeoutMax, setTimeoutMax] = useState(3600);
  // Last persisted value, kept so blur/Enter can no-op when nothing changed.
  const [savedTimeoutSecs, setSavedTimeoutSecs] = useState<number | null>(null);
  const [timeoutError, setTimeoutError] = useState<string | null>(null);
  const [timeoutSavedNote, setTimeoutSavedNote] = useState<string | null>(null);
  const timeoutSeqRef = useRef(0);

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
        const autonomyResp = await openhumanGetAutonomySettings();
        if (cancelled) return;
        setLevel(autonomyResp.result.level);
        setWorkspaceOnly(autonomyResp.result.workspace_only);
        setRequireTaskPlanApproval(autonomyResp.result.require_task_plan_approval ?? true);
        setTrustedRoots(autonomyResp.result.trusted_roots ?? []);
        setAutoApprove(autonomyResp.result.auto_approve ?? []);
      } catch (e) {
        if (!cancelled)
          setError(e instanceof Error ? e.message : t('settings.agentAccess.loadError'));
      } finally {
        if (!cancelled) setIsLoading(false);
      }
      try {
        const agentResp = await openhumanGetAgentSettings();
        if (cancelled) return;
        setTimeoutInput(String(agentResp.result.agent_timeout_secs));
        setSavedTimeoutSecs(agentResp.result.agent_timeout_secs);
        setTimeoutEnvOverride(agentResp.result.env_override);
        setTimeoutMin(agentResp.result.min_timeout_secs);
        setTimeoutMax(agentResp.result.max_timeout_secs);
      } catch {
        // Non-fatal: autonomy controls still render; timeout section
        // stays at defaults and the user can try saving manually.
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
    requireTaskPlanApproval: boolean;
    trustedRoots: TrustedRoot[];
    // Only sent when the allowlist itself is being changed. Omitting it leaves
    // the server's `auto_approve` untouched (partial patch) — important so a
    // tier/folder change here can't clobber a tool the user just added via the
    // in-chat "Always allow" button.
    autoApprove?: string[];
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
        require_task_plan_approval: next.requireTaskPlanApproval,
        ...(next.autoApprove !== undefined ? { auto_approve: next.autoApprove } : {}),
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
    void persist({ level: next, workspaceOnly, requireTaskPlanApproval, trustedRoots });
  };

  const toggleWorkspaceOnly = (next: boolean) => {
    setWorkspaceOnly(next);
    void persist({ level, workspaceOnly: next, requireTaskPlanApproval, trustedRoots });
  };

  const toggleTaskPlanApproval = (next: boolean) => {
    setRequireTaskPlanApproval(next);
    void persist({ level, workspaceOnly, requireTaskPlanApproval: next, trustedRoots });
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
    void persist({ level, workspaceOnly, requireTaskPlanApproval, trustedRoots: nextRoots });
  };

  const removeRoot = (path: string) => {
    const nextRoots = trustedRoots.filter(r => r.path !== path);
    setTrustedRoots(nextRoots);
    void persist({ level, workspaceOnly, requireTaskPlanApproval, trustedRoots: nextRoots });
  };

  const removeAutoApprove = (tool: string) => {
    const nextList = autoApprove.filter(name => name !== tool);
    setAutoApprove(nextList);
    void persist({
      level,
      workspaceOnly,
      requireTaskPlanApproval,
      trustedRoots,
      autoApprove: nextList,
    });
  };

  // Persist the action timeout on blur / Enter. Validates the integer range
  // client-side (the core re-validates) and no-ops when unchanged. Separate
  // from the autonomy `persist` path so a timeout edit can't clobber the
  // autonomy block and vice-versa.
  const commitTimeout = async () => {
    if (!isTauri()) return;
    const trimmed = timeoutInput.trim();
    const parsed = Number(trimmed);
    if (!Number.isInteger(parsed) || parsed < timeoutMin || parsed > timeoutMax) {
      setTimeoutError(`${t('settings.agentAccess.timeout.invalid')} (${timeoutMin}–${timeoutMax})`);
      setTimeoutSavedNote(null);
      return;
    }
    if (savedTimeoutSecs !== null && parsed === savedTimeoutSecs) {
      // Normalize the field (e.g. strip whitespace / leading zeros) but skip the RPC.
      setTimeoutInput(String(parsed));
      setTimeoutError(null);
      return;
    }
    const seq = ++timeoutSeqRef.current;
    const draftAtCommit = timeoutInput;
    setTimeoutError(null);
    setTimeoutSavedNote(null);
    try {
      await openhumanUpdateAgentSettings({ agent_timeout_secs: parsed });
      if (timeoutSeqRef.current === seq) {
        setSavedTimeoutSecs(parsed);
        // Only snap the field value back if the user hasn't typed further.
        if (timeoutInput === draftAtCommit) {
          setTimeoutInput(String(parsed));
        }
        setTimeoutSavedNote(t('settings.agentAccess.saved'));
      }
    } catch (e) {
      if (timeoutSeqRef.current === seq) {
        setTimeoutError(e instanceof Error ? e.message : t('settings.agentAccess.saveError'));
      }
    }
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
          <p className="text-sm text-coral-600 dark:text-coral-300">
            {t('settings.agentAccess.desktopOnly')}
          </p>
        )}

        {isLoading ? (
          <p className="text-sm text-stone-600 dark:text-neutral-400">
            {t('settings.agentAccess.loading')}
          </p>
        ) : (
          <>
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
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
                        ? 'border-primary-500 bg-primary-50 dark:bg-primary-500/10'
                        : 'border-stone-200 dark:border-neutral-800 hover:border-primary-300 dark:hover:border-primary-500'
                    }`}>
                    <div className="flex items-center gap-2">
                      <span
                        className={`inline-block w-3 h-3 rounded-full border ${
                          level === p.id
                            ? 'bg-primary-500 border-primary-500'
                            : 'border-stone-300 dark:border-neutral-700'
                        }`}
                      />
                      <span className="font-medium text-stone-900 dark:text-neutral-100">
                        {p.title}
                      </span>
                      {p.id === 'supervised' && (
                        <span className="text-xs text-stone-600 dark:text-neutral-400">
                          {t('settings.agentAccess.defaultTag')}
                        </span>
                      )}
                    </div>
                    <p className="mt-1 text-xs text-stone-600 dark:text-neutral-400">
                      {p.description}
                    </p>
                  </button>
                ))}
                {level === 'full' && (
                  <p className="rounded border border-coral/40 bg-coral/5 dark:bg-coral/10 p-2 text-xs text-coral-600 dark:text-coral-300">
                    {t('settings.agentAccess.fullWarning')}
                  </p>
                )}
              </div>
            </section>

            {/* Directory model — action sandbox vs internal state. */}
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                {t('settings.agentAccess.directories')}
              </h2>
              <div className="rounded-lg border border-stone-200 dark:border-neutral-800 divide-y divide-stone-200 dark:divide-neutral-800">
                <div className="px-3 py-2">
                  <div className="flex items-center gap-2">
                    <span className="inline-block w-2 h-2 rounded-full bg-sage-500" />
                    <span className="text-xs font-medium text-stone-900 dark:text-neutral-100">
                      {t('settings.agentAccess.actionSandbox')}
                    </span>
                    <span className="text-xs text-sage-600 dark:text-sage-400">
                      {t('settings.agentAccess.readWriteAccess')}
                    </span>
                  </div>
                  <p className="mt-0.5 text-xs text-stone-600 dark:text-neutral-400 font-mono">
                    ~/OpenHuman/projects
                  </p>
                  <p className="text-xs text-stone-500 dark:text-neutral-500">
                    {t('settings.agentAccess.actionSandboxDesc')}
                  </p>
                </div>
                <div className="px-3 py-2">
                  <div className="flex items-center gap-2">
                    <span className="inline-block w-2 h-2 rounded-full bg-coral-500" />
                    <span className="text-xs font-medium text-stone-900 dark:text-neutral-100">
                      {t('settings.agentAccess.internalState')}
                    </span>
                    <span className="text-xs text-coral-600 dark:text-coral-400">
                      {t('settings.agentAccess.agentBlocked')}
                    </span>
                  </div>
                  <p className="mt-0.5 text-xs text-stone-600 dark:text-neutral-400 font-mono">
                    ~/.openhuman/workspace
                  </p>
                  <p className="text-xs text-stone-500 dark:text-neutral-500">
                    {t('settings.agentAccess.internalStateDesc')}
                  </p>
                </div>
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
                  <span className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                    {t('settings.agentAccess.confine.label')}
                  </span>
                  <span className="block text-xs text-stone-600 dark:text-neutral-400">
                    {t('settings.agentAccess.confine.desc')}
                  </span>
                </span>
              </label>
            </section>

            <section className="space-y-1">
              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  className="mt-0.5 cursor-pointer"
                  checked={requireTaskPlanApproval}
                  onChange={e => toggleTaskPlanApproval(e.target.checked)}
                />
                <span>
                  <span className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                    {t('settings.agentAccess.requireTaskPlanApproval.label')}
                  </span>
                  <span className="block text-xs text-stone-600 dark:text-neutral-400">
                    {t('settings.agentAccess.requireTaskPlanApproval.desc')}
                  </span>
                </span>
              </label>
            </section>

            {/* Action timeout — wall-clock limit for a single tool/action.
                Extend it when large local models get cut off mid-response
                (issue #3100). Persists independently of the autonomy block. */}
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                {t('settings.agentAccess.timeout.label')}
              </h2>
              <p className="text-xs text-stone-600 dark:text-neutral-400">
                {t('settings.agentAccess.timeout.desc')}
              </p>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  inputMode="numeric"
                  min={timeoutMin}
                  max={timeoutMax}
                  step={1}
                  value={timeoutInput}
                  disabled={timeoutEnvOverride}
                  onChange={e => setTimeoutInput(e.target.value)}
                  onBlur={() => void commitTimeout()}
                  onKeyDown={e => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      void commitTimeout();
                    }
                  }}
                  aria-label={t('settings.agentAccess.timeout.label')}
                  className="w-28 rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-900 dark:text-neutral-100 px-2 py-1 text-sm disabled:opacity-60"
                />
                <span className="text-xs text-stone-600 dark:text-neutral-400">
                  {t('settings.agentAccess.timeout.unit')} ({timeoutMin}–{timeoutMax})
                </span>
              </div>
              {timeoutEnvOverride && (
                <p className="rounded border border-amber/40 bg-amber/5 dark:bg-amber/10 p-2 text-xs text-amber-700 dark:text-amber-300">
                  {t('settings.agentAccess.timeout.envOverride')}
                </p>
              )}
              <div className="min-h-[1.25rem] text-xs" aria-live="polite">
                {timeoutError ? (
                  <span className="text-coral-600 dark:text-coral-300">{timeoutError}</span>
                ) : timeoutSavedNote ? (
                  <span className="text-sage-700 dark:text-sage-300">✓ {timeoutSavedNote}</span>
                ) : null}
              </div>
            </section>

            {/* Granted folders (trusted roots) — extra read/write reach. */}
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                {t('settings.agentAccess.grantedFolders')}
              </h2>
              <p className="text-xs text-stone-600 dark:text-neutral-400">
                {t('settings.agentAccess.grantedDesc')}
              </p>
              {trustedRoots.length === 0 ? (
                <p className="text-xs text-stone-600 dark:text-neutral-400">
                  {t('settings.agentAccess.noneGranted')}
                </p>
              ) : (
                <ul className="space-y-1">
                  {trustedRoots.map(r => (
                    <li
                      key={r.path}
                      className="flex items-center justify-between rounded border border-stone-200 dark:border-neutral-800 px-2 py-1">
                      <span className="font-mono text-xs text-stone-900 dark:text-neutral-100 truncate">
                        {r.path}
                      </span>
                      <span className="flex items-center gap-2">
                        <span className="text-xs text-stone-600 dark:text-neutral-400">
                          {r.access === 'readwrite'
                            ? t('settings.agentAccess.readWrite')
                            : t('settings.agentAccess.readOnly')}
                        </span>
                        <button
                          type="button"
                          onClick={() => removeRoot(r.path)}
                          className="text-xs text-coral-600 dark:text-coral-300 hover:underline">
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
                  className="flex-1 rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-900 dark:text-neutral-100 px-2 py-1 text-xs font-mono"
                />
                <select
                  value={newRootAccess}
                  onChange={e => setNewRootAccess(e.target.value as TrustedAccess)}
                  aria-label={t('settings.agentAccess.accessLevelLabel')}
                  className="rounded border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-900 dark:text-neutral-100 px-2 py-1 text-xs">
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

            {/* "Always allow" allowlist — tools the user chose to stop being
                prompted for, via the in-chat approval card. Read-only here with
                a Remove action to re-enable prompting for a tool. */}
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                {t('settings.agentAccess.alwaysAllow')}
              </h2>
              <p className="text-xs text-stone-600 dark:text-neutral-400">
                {t('settings.agentAccess.alwaysAllowDesc')}
              </p>
              {autoApprove.length === 0 ? (
                <p className="text-xs text-stone-600 dark:text-neutral-400">
                  {t('settings.agentAccess.alwaysAllowNone')}
                </p>
              ) : (
                <ul className="space-y-1">
                  {autoApprove.map(tool => (
                    <li
                      key={tool}
                      className="flex items-center justify-between rounded border border-stone-200 dark:border-neutral-800 px-2 py-1">
                      <span className="font-mono text-xs text-stone-900 dark:text-neutral-100 truncate">
                        {tool}
                      </span>
                      <button
                        type="button"
                        onClick={() => removeAutoApprove(tool)}
                        className="text-xs text-coral-600 dark:text-coral-300 hover:underline">
                        {t('settings.agentAccess.remove')}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            {/* Approval history — read-only audit trail of past decisions,
                backed by the gate's durable decided-rows store. */}
            <section className="space-y-2">
              <h2 className="text-sm font-semibold text-ink">
                {t('settings.agentAccess.approvalHistory')}
              </h2>
              <p className="text-xs text-ink-soft">
                {t('settings.agentAccess.approvalHistoryDesc')}
              </p>
              <button
                type="button"
                onClick={() => navigateToSettings('approval-history')}
                data-testid="agent-access-approval-history-link"
                className="rounded border border-line px-3 py-1 text-xs text-ink hover:border-primary-300">
                {t('settings.agentAccess.viewApprovalHistory')}
              </button>
            </section>

            {/* Auto-save status — changes persist on selection; no manual save. */}
            <div className="min-h-[1.25rem] text-sm" aria-live="polite">
              {error ? (
                <span className="text-coral-600 dark:text-coral-300">{error}</span>
              ) : isSaving ? (
                <span className="text-stone-600 dark:text-neutral-400">
                  {t('settings.agentAccess.saving')}
                </span>
              ) : savedNote ? (
                <span className="text-sage-700 dark:text-sage-300">✓ {savedNote}</span>
              ) : (
                <span className="text-stone-600 dark:text-neutral-400">
                  {t('settings.agentAccess.changesApply')}
                </span>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
};

export default AgentAccessPanel;
