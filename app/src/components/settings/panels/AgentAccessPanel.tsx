import { ChevronRight, Plus } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type AutonomyLevel,
  openhumanGetAgentSettings,
  openhumanGetAutonomySettings,
  openhumanUpdateAgentSettings,
  openhumanUpdateAutonomySettings,
  type ToolDispatcher,
  type TrustedAccess,
  type TrustedRoot,
} from '../../../utils/tauriCommands';
import { Alert, AlertDescription, Button, Card, Field } from '../../ui';
import {
  SettingsBadge,
  SettingsEmptyState,
  SettingsListItem,
  SettingsNumberField,
  SettingsSelect,
  SettingsStatusLine,
  SettingsSwitch,
  SettingsTextField,
} from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsPanel from '../layout/SettingsPanel';
import AutonomyRateLimitSection from './AutonomyPanel';
import FilesFolderSection from './FilesFolderSection';

// Kept false for compatibility with older config shapes. Host package-manager
// execution is unavailable until it can run in an approved isolated environment.
const ALLOW_TOOL_INSTALL = false;

const AgentAccessPanel = () => {
  const { t } = useT();
  const { navigateToSettings } = useSettingsNavigation();

  // Load `level` so we can carry it through when writing other fields, but
  // the tier-selection UI lives in PermissionsPanel. Never render tier radios
  // here — that would create two sources of truth.
  const [level, setLevel] = useState<AutonomyLevel>('supervised');
  const [workspaceOnly, setWorkspaceOnly] = useState(false);
  // Blanket "auto-approve everything" bypass — off by default. Hard security
  // blocks (credential dirs, workspace-internal paths) and the
  // unlabelled-origin denial in the approval gate are unaffected by this setting; see `settings.agentAccess.autoApproveAll.desc`.
  const [autoApproveAll, setAutoApproveAll] = useState(false);
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

  // Tool-call dialect (`agent.tool_dispatcher`). `auto` = JSON, the default.
  const [toolDispatcher, setToolDispatcher] = useState<ToolDispatcher>('auto');
  const [toolDispatcherEnvOverride, setToolDispatcherEnvOverride] = useState(false);
  const [toolDispatcherError, setToolDispatcherError] = useState<string | null>(null);
  const [toolDispatcherSavedNote, setToolDispatcherSavedNote] = useState<string | null>(null);
  const toolDispatcherSeqRef = useRef(0);

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
      try {
        const autonomyResp = await openhumanGetAutonomySettings();
        if (cancelled) return;
        setLevel(autonomyResp.result.level);
        setWorkspaceOnly(autonomyResp.result.workspace_only);
        setAutoApproveAll(autonomyResp.result.auto_approve_all ?? false);
        setTrustedRoots(autonomyResp.result.trusted_roots ?? []);
        setAutoApprove(autonomyResp.result.auto_approve ?? []);
      } catch (e) {
        if (!cancelled)
          setError(e instanceof Error ? e.message : t('settings.agentAccess.loadError'));
      }
      try {
        const agentResp = await openhumanGetAgentSettings();
        if (cancelled) return;
        setTimeoutInput(String(agentResp.result.agent_timeout_secs));
        setSavedTimeoutSecs(agentResp.result.agent_timeout_secs);
        setTimeoutEnvOverride(agentResp.result.env_override);
        setTimeoutMin(agentResp.result.min_timeout_secs);
        setTimeoutMax(agentResp.result.max_timeout_secs);
        setToolDispatcher(agentResp.result.tool_dispatcher ?? 'auto');
        setToolDispatcherEnvOverride(agentResp.result.tool_dispatcher_env_override ?? false);
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
  //
  // `onError` lets a caller revert its own optimistic `setState` if the RPC
  // fails — otherwise a
  // failed save leaves the switch showing the new value locally while the
  // server-side field silently kept its old one.
  const persist = async (
    next: {
      workspaceOnly: boolean;
      trustedRoots: TrustedRoot[];
      // Only sent when the allowlist itself is being changed. Omitting it leaves
      // the server's `auto_approve` untouched (partial patch) — important so a
      // tier/folder change can't clobber a tool the user just added via the
      // in-chat "Always allow" button.
      autoApprove?: string[];
      // Same partial-patch reasoning as `autoApprove` above: only
      // `toggleAutoApproveAll` sets this. Every other caller must omit it so
      // an unrelated autosave (folders, workspace confinement) can never
      // rewrite `auto_approve_all` back to this
      // panel's possibly-stale local value.
      autoApproveAll?: boolean;
    },
    onError?: () => void
  ) => {
    const seq = ++persistSeqRef.current;
    setError(null);
    setSavedNote(null);
    setIsSaving(true);
    try {
      await openhumanUpdateAutonomySettings({
        level,
        workspace_only: next.workspaceOnly,
        trusted_roots: next.trustedRoots,
        allow_tool_install: ALLOW_TOOL_INSTALL,
        ...(next.autoApprove !== undefined ? { auto_approve: next.autoApprove } : {}),
        ...(next.autoApproveAll !== undefined ? { auto_approve_all: next.autoApproveAll } : {}),
      });
      // Only the most recent persist may write UI state back.
      if (persistSeqRef.current === seq) {
        setSavedNote(t('settings.agentAccess.saved'));
      }
    } catch (e) {
      if (persistSeqRef.current === seq) {
        setError(e instanceof Error ? e.message : t('settings.agentAccess.saveError'));
        onError?.();
      }
    } finally {
      if (persistSeqRef.current === seq) {
        setIsSaving(false);
      }
    }
  };

  // Persist the tool-call format on change; revert the select if the save fails.
  const changeToolDispatcher = async (next: ToolDispatcher) => {
    const prev = toolDispatcher;
    const seq = ++toolDispatcherSeqRef.current;
    setToolDispatcher(next);
    setToolDispatcherError(null);
    setToolDispatcherSavedNote(null);
    try {
      await openhumanUpdateAgentSettings({ tool_dispatcher: next });
      if (toolDispatcherSeqRef.current === seq) {
        setToolDispatcherSavedNote(t('settings.agentAccess.saved'));
      }
    } catch (e) {
      if (toolDispatcherSeqRef.current === seq) {
        setToolDispatcher(prev);
        setToolDispatcherError(
          e instanceof Error ? e.message : t('settings.agentAccess.saveError')
        );
      }
    }
  };

  const toggleWorkspaceOnly = (next: boolean) => {
    const prev = workspaceOnly;
    setWorkspaceOnly(next);
    void persist({ workspaceOnly: next, trustedRoots }, () => setWorkspaceOnly(prev));
  };

  const toggleAutoApproveAll = (next: boolean) => {
    const prev = autoApproveAll;
    setAutoApproveAll(next);
    void persist({ workspaceOnly, trustedRoots, autoApproveAll: next }, () =>
      setAutoApproveAll(prev)
    );
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
    // `autoApproveAll` intentionally omitted: this save is about the folder
    // grant, not the auto-approve-all toggle, and the partial-patch RPC
    // leaves omitted fields untouched server-side (see `persist` above).
    void persist({ workspaceOnly, trustedRoots: nextRoots });
  };

  const removeRoot = (path: string) => {
    const nextRoots = trustedRoots.filter(r => r.path !== path);
    setTrustedRoots(nextRoots);
    // `autoApproveAll` intentionally omitted — see `addRoot` above.
    void persist({ workspaceOnly, trustedRoots: nextRoots });
  };

  const removeAutoApprove = (tool: string) => {
    const nextList = autoApprove.filter(name => name !== tool);
    setAutoApprove(nextList);
    // `autoApproveAll` intentionally omitted — see `addRoot` above.
    void persist({ workspaceOnly, trustedRoots, autoApprove: nextList });
  };

  // Persist the action timeout on blur / Enter. Validates the integer range
  // client-side (the core re-validates) and no-ops when unchanged. Separate
  // from the autonomy `persist` path so a timeout edit can't clobber the
  // autonomy block and vice-versa.
  const commitTimeout = async () => {
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
      {isLoading ? (
        <p className="text-sm text-content-muted">{t('settings.agentAccess.loading')}</p>
      ) : (
        <>
          {/* ── Approvals: when the agent must ask first ─────────────────── */}
          <Card title={t('settings.agentAccess.group.approvals')}>
            {/* Auto-approve everything — blanket bypass of the approval
                prompt. Security-sensitive: first on the page, with a
                persistent warning visible regardless of toggle state. */}
            <div>
              <Field
                htmlFor="switch-auto-approve-all"
                label={t('settings.agentAccess.autoApproveAll.label')}
                control={
                  <SettingsSwitch
                    id="switch-auto-approve-all"
                    checked={autoApproveAll}
                    onCheckedChange={toggleAutoApproveAll}
                    aria-label={t('settings.agentAccess.autoApproveAll.label')}
                  />
                }
              />
              <div className="-mt-1 px-4 pb-3">
                {/* Persistent, not a response to a user action — so no
                    assertive announcement on every visit. */}
                <Alert
                  variant="warning"
                  density="compact"
                  role={undefined}
                  data-testid="auto-approve-all-warning">
                  <AlertDescription>
                    {t('settings.agentAccess.autoApproveAll.desc')}
                  </AlertDescription>
                </Alert>
              </div>
            </div>

            {/* Always-allowed tools */}
            <div>
              <div className="px-4 pt-3">
                <div className="text-sm font-medium text-content">
                  {t('settings.agentAccess.alwaysAllow')}
                </div>
                <p className="mt-0.5 text-xs leading-relaxed text-content-muted">
                  {t('settings.agentAccess.alwaysAllowDesc')}
                </p>
              </div>
              {autoApprove.length === 0 ? (
                <SettingsEmptyState label={t('settings.agentAccess.alwaysAllowNone')} />
              ) : (
                <ul className="py-1">
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
            </div>

            <Field
              label={t('settings.agentAccess.approvalHistory')}
              description={t('settings.agentAccess.approvalHistoryDesc')}
              control={
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  trailingIcon={<ChevronRight className="h-3.5 w-3.5" aria-hidden />}
                  onClick={() => navigateToSettings('approval-history')}
                  data-testid="agent-access-approval-history-link">
                  {t('settings.agentAccess.viewApprovalHistory')}
                </Button>
              }
            />
          </Card>

          {/* ── File system: where the agent may read and write ─────────── */}
          <Card title={t('settings.agentAccess.group.fileSystem')}>
            <FilesFolderSection />

            <Field
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

            {/* Granted folders (trusted roots) */}
            <div>
              <div className="px-4 pt-3">
                <div className="text-sm font-medium text-content">
                  {t('settings.agentAccess.grantedFolders')}
                </div>
                <p className="mt-0.5 text-xs leading-relaxed text-content-muted">
                  {t('settings.agentAccess.grantedDesc')}
                </p>
              </div>
              {trustedRoots.length === 0 ? (
                <SettingsEmptyState label={t('settings.agentAccess.noneGranted')} />
              ) : (
                <ul className="py-1">
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
              <div className="flex items-center gap-2 px-4 pb-4 pt-1">
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
                  variant="secondary"
                  size="sm"
                  leadingIcon={<Plus className="h-3.5 w-3.5" aria-hidden />}
                  onClick={addRoot}
                  disabled={!newRootPath.trim()}>
                  {t('settings.agentAccess.add')}
                </Button>
              </div>
            </div>
          </Card>

          {/* ── Limits ───────────────────────────────────────────────────── */}
          <Card title={t('settings.agentAccess.group.limits')}>
            <div>
              <Field
                label={t('settings.agentAccess.timeout.label')}
                description={t('settings.agentAccess.timeout.desc')}
                control={
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
                }
              />
              {(timeoutEnvOverride || timeoutSavedNote || timeoutError) && (
                <div className="space-y-2 px-4 pb-3">
                  {timeoutEnvOverride && (
                    // Reflects a resolved config value, not a user action.
                    <Alert variant="warning" density="compact" role={undefined}>
                      <AlertDescription>
                        {t('settings.agentAccess.timeout.envOverride')}
                      </AlertDescription>
                    </Alert>
                  )}
                  <SettingsStatusLine
                    saving={false}
                    savedNote={timeoutSavedNote}
                    error={timeoutError}
                    savingLabel={t('settings.agentAccess.saving')}
                  />
                </div>
              )}
            </div>
          </Card>

          {/* ── Tool-call format: JSON by default, code styles opt-in ────── */}
          <Card title={t('settings.agentAccess.toolFormat.label')}>
            <Field
              htmlFor="tool-dispatcher-select"
              label={t('settings.agentAccess.toolFormat.label')}
              description={t('settings.agentAccess.toolFormat.desc')}
              control={
                <SettingsSelect
                  id="tool-dispatcher-select"
                  value={toolDispatcher}
                  onChange={e => void changeToolDispatcher(e.target.value as ToolDispatcher)}
                  disabled={toolDispatcherEnvOverride}
                  aria-label={t('settings.agentAccess.toolFormat.label')}
                  inputSize="sm"
                  className="w-64">
                  <option value="auto">{t('settings.agentAccess.toolFormat.option.auto')}</option>
                  <option value="native">
                    {t('settings.agentAccess.toolFormat.option.native')}
                  </option>
                  <option value="xml">{t('settings.agentAccess.toolFormat.option.xml')}</option>
                  <option value="pformat">
                    {t('settings.agentAccess.toolFormat.option.pformat')}
                  </option>
                  <option value="python">
                    {t('settings.agentAccess.toolFormat.option.python')}
                  </option>
                  <option value="typescript">
                    {t('settings.agentAccess.toolFormat.option.typescript')}
                  </option>
                </SettingsSelect>
              }
            />
            {(toolDispatcherEnvOverride || toolDispatcherSavedNote || toolDispatcherError) && (
              <div className="space-y-2 px-4 pb-3">
                {toolDispatcherEnvOverride && (
                  <Alert variant="warning" density="compact" role={undefined}>
                    <AlertDescription>
                      {t('settings.agentAccess.toolFormat.envOverride')}
                    </AlertDescription>
                  </Alert>
                )}
                <SettingsStatusLine
                  saving={false}
                  savedNote={toolDispatcherSavedNote}
                  error={toolDispatcherError}
                  savingLabel={t('settings.agentAccess.saving')}
                />
              </div>
            )}
          </Card>

          {/* Action rate limit (formerly the standalone /settings/autonomy page) */}
          <AutonomyRateLimitSection />

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
