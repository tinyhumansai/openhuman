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
import { openhumanCronList, openhumanCronUpdate } from '../../../utils/tauriCommands/cron';
import Button from '../../ui/Button';
import {
  SettingsBadge,
  SettingsEmptyState,
  SettingsListItem,
  SettingsNumberField,
  SettingsRow,
  SettingsSection,
  SettingsSelect,
  SettingsStatusLine,
  SettingsSwitch,
  SettingsTextField,
} from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';
import AutonomyRateLimitSection from './AutonomyPanel';

// Installs are always *available* but never silent: every `install_tool` call
// is routed through the approval gate, so the user is asked to Approve/Deny
// each install in chat. There is therefore no per-user "disable installs" knob
// here — the consent is captured per-install by the gate, not by a static
// config flag.
const ALLOW_TOOL_INSTALL = true;

const AgentAccessPanel = () => {
  const { t } = useT();
  const { navigateToSettings } = useSettingsNavigation();

  // Load `level` so we can carry it through when writing other fields, but
  // the tier-selection UI lives in PermissionsPanel. Never render tier radios
  // here — that would create two sources of truth.
  const [level, setLevel] = useState<AutonomyLevel>('supervised');
  const [workspaceOnly, setWorkspaceOnly] = useState(false);
  const [requireTaskPlanApproval, setRequireTaskPlanApproval] = useState(true);
  const [trustedRoots, setTrustedRoots] = useState<TrustedRoot[]>([]);
  // "Always allow" allowlist — populated by the in-chat "Always allow" button;
  // shown here read-only with a Remove action (the re-protect path).
  const [autoApprove, setAutoApprove] = useState<string[]>([]);

  const [newRootPath, setNewRootPath] = useState('');
  const [newRootAccess, setNewRootAccess] = useState<TrustedAccess>('read');

  // Autonomous tiny.place agent ("autopilot") — a seeded, *disabled* cron job
  // the user opts into here. It's not an autonomy field: we resolve its id by
  // name from the cron list and flip its `enabled` flag via cron_update. The
  // section only renders once the job is found (id known).
  const [autopilotJobId, setAutopilotJobId] = useState<string | null>(null);
  const [autopilotEnabled, setAutopilotEnabled] = useState(false);
  // Monotonic guard so rapid toggles can't resolve out-of-order and leave the
  // UI showing a stale enabled state (last write wins).
  const autopilotSeqRef = useRef(0);

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
      }
      try {
        // Resolve the seeded tinyplace_autopilot cron job by name so the toggle
        // below can flip its enabled flag. Non-fatal: the section just stays
        // hidden if the job isn't present or the list call fails.
        const cronResp = await openhumanCronList();
        if (cancelled) return;
        const autopilot = cronResp.result.find(j => j.name === 'tinyplace_autopilot');
        if (autopilot) {
          setAutopilotJobId(autopilot.id);
          setAutopilotEnabled(autopilot.enabled);
        }
      } catch {
        // Non-fatal — bounty-worker toggle stays hidden.
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
  // `allow_tool_install` is fixed; workspace_only, trusted_roots vary.
  // `level` is carried through from state (its UI lives in PermissionsPanel).
  // Pass explicit `next` values (setState is async).
  const persist = async (next: {
    workspaceOnly: boolean;
    requireTaskPlanApproval: boolean;
    trustedRoots: TrustedRoot[];
    // Only sent when the allowlist itself is being changed. Omitting it leaves
    // the server's `auto_approve` untouched (partial patch) — important so a
    // tier/folder change can't clobber a tool the user just added via the
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
        level,
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

  const toggleWorkspaceOnly = (next: boolean) => {
    setWorkspaceOnly(next);
    void persist({ workspaceOnly: next, requireTaskPlanApproval, trustedRoots });
  };

  const toggleTaskPlanApproval = (next: boolean) => {
    setRequireTaskPlanApproval(next);
    void persist({ workspaceOnly, requireTaskPlanApproval: next, trustedRoots });
  };

  // The autopilot is a cron job, not an autonomy field — flip its `enabled`
  // flag directly via cron_update. Optimistic, with revert on failure, and a
  // sequence guard so only the most recent toggle writes UI state back.
  const toggleAutopilot = async (next: boolean) => {
    if (!autopilotJobId || !isTauri()) return;
    const seq = ++autopilotSeqRef.current;
    const prev = autopilotEnabled;
    setAutopilotEnabled(next);
    setError(null);
    setSavedNote(null);
    try {
      await openhumanCronUpdate(autopilotJobId, { enabled: next });
      if (autopilotSeqRef.current === seq) {
        setSavedNote(t('settings.agentAccess.saved'));
      }
    } catch (e) {
      if (autopilotSeqRef.current === seq) {
        setAutopilotEnabled(prev);
        setError(e instanceof Error ? e.message : t('settings.agentAccess.saveError'));
      }
    }
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
    void persist({ workspaceOnly, requireTaskPlanApproval, trustedRoots: nextRoots });
  };

  const removeRoot = (path: string) => {
    const nextRoots = trustedRoots.filter(r => r.path !== path);
    setTrustedRoots(nextRoots);
    void persist({ workspaceOnly, requireTaskPlanApproval, trustedRoots: nextRoots });
  };

  const removeAutoApprove = (tool: string) => {
    const nextList = autoApprove.filter(name => name !== tool);
    setAutoApprove(nextList);
    void persist({ workspaceOnly, requireTaskPlanApproval, trustedRoots, autoApprove: nextList });
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
    <SettingsPanel description={t('settings.agentAccess.menuDesc')}>
      {/* Desktop-only notice */}
      {!isTauri() && (
        <p className="text-sm text-coral-600 dark:text-coral-300">
          {t('settings.agentAccess.desktopOnly')}
        </p>
      )}

      {isLoading ? (
        <p className="text-sm text-content-muted">{t('settings.agentAccess.loading')}</p>
      ) : (
        <>
          {/* Workspace confinement + task plan approval */}
          <SettingsSection>
            <SettingsRow
              htmlFor="switch-workspace-only"
              label={t('settings.agentAccess.confine.label')}
              description={t('settings.agentAccess.confine.desc')}
              control={
                <SettingsSwitch
                  id="switch-workspace-only"
                  checked={workspaceOnly}
                  onCheckedChange={toggleWorkspaceOnly}
                  aria-label={t('settings.agentAccess.confine.label')}
                />
              }
            />
            <SettingsRow
              htmlFor="switch-task-plan-approval"
              label={t('settings.agentAccess.requireTaskPlanApproval.label')}
              description={t('settings.agentAccess.requireTaskPlanApproval.desc')}
              control={
                <SettingsSwitch
                  id="switch-task-plan-approval"
                  checked={requireTaskPlanApproval}
                  onCheckedChange={toggleTaskPlanApproval}
                  aria-label={t('settings.agentAccess.requireTaskPlanApproval.label')}
                />
              }
            />
          </SettingsSection>

          {/* Autonomous tiny.place agent (opt-in). Only shown once the seeded
                cron job is found, so users without it never see a dead toggle. */}
          {autopilotJobId && (
            <SettingsSection
              title={t('settings.agentAccess.tinyplaceAutopilot.title')}
              description={t('settings.agentAccess.tinyplaceAutopilot.desc')}>
              <SettingsRow
                htmlFor="switch-tinyplace-autopilot"
                label={t('settings.agentAccess.tinyplaceAutopilot.label')}
                control={
                  <SettingsSwitch
                    id="switch-tinyplace-autopilot"
                    checked={autopilotEnabled}
                    onCheckedChange={next => void toggleAutopilot(next)}
                    aria-label={t('settings.agentAccess.tinyplaceAutopilot.label')}
                  />
                }
              />
            </SettingsSection>
          )}

          {/* Action timeout */}
          <SettingsSection
            title={t('settings.agentAccess.timeout.label')}
            description={t('settings.agentAccess.timeout.desc')}>
            <SettingsRow
              stacked
              control={
                <div className="space-y-2">
                  <SettingsNumberField
                    id="timeout-input"
                    value={timeoutInput}
                    onChange={setTimeoutInput}
                    onCommit={() => void commitTimeout()}
                    unit={t('settings.agentAccess.timeout.unit')}
                    min={timeoutMin}
                    max={timeoutMax}
                    disabled={timeoutEnvOverride}
                    invalid={!!timeoutError}
                    aria-label={t('settings.agentAccess.timeout.label')}
                  />
                  {timeoutEnvOverride && (
                    <p className="rounded border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-2 text-xs text-amber-700 dark:text-amber-300">
                      {t('settings.agentAccess.timeout.envOverride')}
                    </p>
                  )}
                  <SettingsStatusLine
                    saving={false}
                    savedNote={timeoutSavedNote}
                    error={timeoutError}
                    savingLabel={t('settings.agentAccess.saving')}
                  />
                </div>
              }
            />
          </SettingsSection>

          {/* Granted folders (trusted roots) */}
          <SettingsSection
            title={t('settings.agentAccess.grantedFolders')}
            description={t('settings.agentAccess.grantedDesc')}>
            {trustedRoots.length === 0 ? (
              <SettingsEmptyState label={t('settings.agentAccess.noneGranted')} />
            ) : (
              <ul>
                {trustedRoots.map(r => (
                  <SettingsListItem
                    key={r.path}
                    label={r.path}
                    mono
                    badge={
                      r.access === 'readwrite' ? (
                        <SettingsBadge variant="success">
                          {t('settings.agentAccess.readWrite')}
                        </SettingsBadge>
                      ) : (
                        <SettingsBadge variant="neutral">
                          {t('settings.agentAccess.readOnly')}
                        </SettingsBadge>
                      )
                    }
                    onRemove={() => removeRoot(r.path)}
                    removeLabel={t('settings.agentAccess.remove')}
                  />
                ))}
              </ul>
            )}
            {/* Add-folder row */}
            <div className="flex items-center gap-2 px-4 py-3 border-t border-line-subtle">
              <SettingsTextField
                mono
                className="flex-1"
                value={newRootPath}
                onChange={e => setNewRootPath(e.target.value)}
                placeholder={t('settings.agentAccess.pathPlaceholder')}
                aria-label={t('settings.agentAccess.pathPlaceholder')}
                onKeyDown={e => {
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    addRoot();
                  }
                }}
                inputSize="sm"
              />
              <SettingsSelect
                value={newRootAccess}
                onChange={e => setNewRootAccess(e.target.value as TrustedAccess)}
                aria-label={t('settings.agentAccess.accessLevelLabel')}
                inputSize="sm"
                className="w-32">
                <option value="read">{t('settings.agentAccess.readOnly')}</option>
                <option value="readwrite">{t('settings.agentAccess.readWrite')}</option>
              </SettingsSelect>
              <Button
                type="button"
                variant="primary"
                size="xs"
                onClick={addRoot}
                disabled={!newRootPath.trim()}>
                {t('settings.agentAccess.add')}
              </Button>
            </div>
          </SettingsSection>

          {/* Always-allowed tools */}
          <SettingsSection
            title={t('settings.agentAccess.alwaysAllow')}
            description={t('settings.agentAccess.alwaysAllowDesc')}>
            {autoApprove.length === 0 ? (
              <SettingsEmptyState label={t('settings.agentAccess.alwaysAllowNone')} />
            ) : (
              <ul>
                {autoApprove.map(tool => (
                  <SettingsListItem
                    key={tool}
                    label={tool}
                    mono
                    onRemove={() => removeAutoApprove(tool)}
                    removeLabel={t('settings.agentAccess.remove')}
                  />
                ))}
              </ul>
            )}
          </SettingsSection>

          {/* Action rate limit (formerly the standalone /settings/autonomy page) */}
          <AutonomyRateLimitSection />

          {/* Approval history */}
          <SettingsSection
            title={t('settings.agentAccess.approvalHistory')}
            description={t('settings.agentAccess.approvalHistoryDesc')}>
            <div className="px-4 py-3">
              <Button
                type="button"
                variant="secondary"
                size="xs"
                onClick={() => navigateToSettings('approval-history')}
                data-testid="agent-access-approval-history-link">
                {t('settings.agentAccess.viewApprovalHistory')}
              </Button>
            </div>
          </SettingsSection>

          {/* Auto-save status */}
          <SettingsStatusLine
            saving={isSaving}
            savedNote={savedNote}
            error={error}
            savingLabel={t('settings.agentAccess.saving')}
          />
        </>
      )}
    </SettingsPanel>
  );
};

export default AgentAccessPanel;
