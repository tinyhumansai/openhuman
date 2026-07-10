import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import { SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

type ToolPolicyDiagnostics = {
  total_tools: number;
  enabled_tools: number;
  mcp_stdio_tools: number;
  json_rpc_tools: number;
  possible_write_surfaces: string[];
  policy_surfaces: string[];
  posture: {
    autonomy_level: string;
    workspace_only: boolean;
    max_actions_per_hour: number;
    require_approval_for_medium_risk: boolean;
    block_high_risk_commands: boolean;
  };
  mcp_allowlists: {
    enabled: boolean;
    server_count: number;
    enabled_server_count: number;
    servers: {
      name: string;
      enabled: boolean;
      allowed_tools_count: number;
      disallowed_tools_count: number;
      has_allowlist: boolean;
      has_denylist: boolean;
    }[];
  };
  mcp_write_audit: { enabled: boolean; recent_rows: number | null; last_error: string | null };
  recent_denials: {
    timestamp_ms: number;
    tool_name: string;
    policy: string;
    action: string;
    reason: string;
  }[];
};

const ToolPolicyDiagnosticsPanel = () => {
  const { t } = useT();

  const [status, setStatus] = useState<
    | { kind: 'loading' }
    | { kind: 'ready'; diagnostics: ToolPolicyDiagnostics }
    | { kind: 'error'; message: string }
  >({ kind: 'loading' });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const diagnostics = await callCoreRpc<ToolPolicyDiagnostics>({
          method: 'openhuman.tool_registry_diagnostics',
          params: {},
          timeoutMs: 10_000,
        });
        if (cancelled) return;
        setStatus({ kind: 'ready', diagnostics });
      } catch (err) {
        if (cancelled) return;
        setStatus({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const body = useMemo(() => {
    if (status.kind === 'loading') {
      return (
        <div className="px-4 py-3 text-sm text-content-muted">
          {t('devOptions.toolPolicyDiagnostics.loading')}
        </div>
      );
    }
    if (status.kind === 'error') {
      return (
        <div className="px-4 py-3">
          <div className="text-sm font-semibold text-content mb-1">
            {t('devOptions.toolPolicyDiagnostics.unavailable')}
          </div>
          <SettingsStatusLine saving={false} error={status.message} savingLabel="" />
        </div>
      );
    }

    const d = status.diagnostics;
    const recentRows =
      d.mcp_write_audit.recent_rows === null ? '—' : String(d.mcp_write_audit.recent_rows);

    return (
      <div className="px-4 pt-3 pb-6 space-y-3">
        <div className="px-4 py-3 rounded-xl border border-sage-300 dark:border-sage-500/40 bg-sage-50 dark:bg-sage-500/10">
          <div className="text-sm font-semibold text-sage-900 dark:text-sage-200">
            {t('devOptions.toolPolicyDiagnostics.posture.title')}
          </div>
          <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
            <dt className="text-sage-700 dark:text-sage-300">
              {t('devOptions.toolPolicyDiagnostics.posture.autonomy')}
            </dt>
            <dd className="font-mono text-sage-900 dark:text-sage-200">
              {d.posture.autonomy_level}
            </dd>
            <dt className="text-sage-700 dark:text-sage-300">
              {t('devOptions.toolPolicyDiagnostics.posture.workspaceOnly')}
            </dt>
            <dd className="text-sage-900 dark:text-sage-200">{String(d.posture.workspace_only)}</dd>
            <dt className="text-sage-700 dark:text-sage-300">
              {t('devOptions.toolPolicyDiagnostics.posture.maxActionsPerHour')}
            </dt>
            <dd className="font-mono text-sage-900 dark:text-sage-200">
              {d.posture.max_actions_per_hour}
            </dd>
            <dt className="text-sage-700 dark:text-sage-300">
              {t('devOptions.toolPolicyDiagnostics.posture.approvalMediumRisk')}
            </dt>
            <dd className="text-sage-900 dark:text-sage-200">
              {String(d.posture.require_approval_for_medium_risk)}
            </dd>
            <dt className="text-sage-700 dark:text-sage-300">
              {t('devOptions.toolPolicyDiagnostics.posture.blockHighRisk')}
            </dt>
            <dd className="text-sage-900 dark:text-sage-200">
              {String(d.posture.block_high_risk_commands)}
            </dd>
          </dl>
        </div>

        <div className="px-4 py-3 rounded-xl border border-sage-300 dark:border-sage-500/40 bg-surface dark:bg-sage-900/20">
          <div className="text-sm font-semibold text-sage-900 dark:text-sage-200">
            {t('devOptions.toolPolicyDiagnostics.inventory.title')}
          </div>
          <dl className="mt-2 grid grid-cols-2 gap-x-6 gap-y-1 text-xs">
            <div>
              <dt className="text-sage-700 dark:text-sage-300">
                {t('devOptions.toolPolicyDiagnostics.inventory.totalTools')}
              </dt>
              <dd className="font-mono text-sage-900 dark:text-sage-200">{d.total_tools}</dd>
            </div>
            <div>
              <dt className="text-sage-700 dark:text-sage-300">
                {t('devOptions.toolPolicyDiagnostics.inventory.enabledTools')}
              </dt>
              <dd className="font-mono text-sage-900 dark:text-sage-200">{d.enabled_tools}</dd>
            </div>
            <div>
              <dt className="text-sage-700 dark:text-sage-300">
                {t('devOptions.toolPolicyDiagnostics.inventory.mcpStdioTools')}
              </dt>
              <dd className="font-mono text-sage-900 dark:text-sage-200">{d.mcp_stdio_tools}</dd>
            </div>
            <div>
              <dt className="text-sage-700 dark:text-sage-300">
                {t('devOptions.toolPolicyDiagnostics.inventory.jsonRpcTools')}
              </dt>
              <dd className="font-mono text-sage-900 dark:text-sage-200">{d.json_rpc_tools}</dd>
            </div>
          </dl>
        </div>

        <div className="px-4 py-3 rounded-xl border border-sage-300 dark:border-sage-500/40 bg-surface dark:bg-sage-900/20">
          <div className="text-sm font-semibold text-sage-900 dark:text-sage-200">
            {t('devOptions.toolPolicyDiagnostics.mcpAllowlists.title')}
          </div>
          <div className="mt-1 text-xs text-sage-700 dark:text-sage-300">
            {t('devOptions.toolPolicyDiagnostics.mcpAllowlists.summary')
              .replace('{enabled}', String(d.mcp_allowlists.enabled))
              .replace('{enabledCount}', String(d.mcp_allowlists.enabled_server_count))
              .replace('{totalCount}', String(d.mcp_allowlists.server_count))}
          </div>
          {d.mcp_allowlists.servers.length > 0 && (
            <ul className="mt-2 text-xs space-y-1">
              {d.mcp_allowlists.servers.slice(0, 10).map(s => (
                <li key={s.name} className="flex items-center justify-between gap-3">
                  <span
                    className="font-mono text-sage-900 dark:text-sage-200 truncate"
                    title={s.name}>
                    {s.name || t('devOptions.toolPolicyDiagnostics.mcpAllowlists.unnamed')}
                  </span>
                  <span className="text-sage-700 dark:text-sage-300 font-mono">
                    {t('devOptions.toolPolicyDiagnostics.mcpAllowlists.allowDeny')
                      .replace('{allowCount}', String(s.allowed_tools_count))
                      .replace('{denyCount}', String(s.disallowed_tools_count))}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="px-4 py-3 rounded-xl border border-sage-300 dark:border-sage-500/40 bg-surface dark:bg-sage-900/20">
          <div className="text-sm font-semibold text-sage-900 dark:text-sage-200">
            {t('devOptions.toolPolicyDiagnostics.mcpWriteAudit.title')}
          </div>
          <div className="mt-1 text-xs text-sage-700 dark:text-sage-300">
            {t('devOptions.toolPolicyDiagnostics.mcpWriteAudit.summary')
              .replace('{enabled}', String(d.mcp_write_audit.enabled))
              .replace('{recentRows}', recentRows)}
          </div>
          {d.mcp_write_audit.last_error && (
            <div className="mt-2 text-xs text-coral-700 dark:text-coral-200 font-mono break-words">
              {d.mcp_write_audit.last_error}
            </div>
          )}
        </div>

        <div className="px-4 py-3 rounded-xl border border-sage-300 dark:border-sage-500/40 bg-surface dark:bg-sage-900/20">
          <div className="text-sm font-semibold text-sage-900 dark:text-sage-200">
            {t('devOptions.toolPolicyDiagnostics.recentBlocked.title')}
          </div>
          {d.recent_denials.length === 0 ? (
            <div className="mt-1 text-xs text-sage-700 dark:text-sage-300">
              {t('devOptions.toolPolicyDiagnostics.recentBlocked.empty')}
            </div>
          ) : (
            <ul className="mt-2 text-xs space-y-1">
              {d.recent_denials.slice(0, 10).map(entry => (
                <li
                  key={`${entry.timestamp_ms}:${entry.tool_name}`}
                  className="flex flex-col gap-0.5">
                  <div className="flex items-center justify-between gap-3">
                    <span
                      className="font-mono text-sage-900 dark:text-sage-200 truncate"
                      title={entry.tool_name}>
                      {entry.tool_name}
                    </span>
                    <span className="text-sage-700 dark:text-sage-300 font-mono">
                      {entry.policy}:{entry.action}
                    </span>
                  </div>
                  <div className="text-sage-700 dark:text-sage-300 break-words">{entry.reason}</div>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="px-4 py-3 rounded-xl border border-sage-300 dark:border-sage-500/40 bg-surface dark:bg-sage-900/20">
          <div className="text-sm font-semibold text-sage-900 dark:text-sage-200">
            {t('devOptions.toolPolicyDiagnostics.redactedSurfaces.title')}
          </div>
          <div className="mt-1 text-xs text-sage-700 dark:text-sage-300">
            {t('devOptions.toolPolicyDiagnostics.redactedSurfaces.summary')
              .replace('{writeCount}', String(d.possible_write_surfaces.length))
              .replace('{policyCount}', String(d.policy_surfaces.length))}
          </div>
        </div>
      </div>
    );
  }, [status, t]);

  return (
    <SettingsPanel description={t('devOptions.toolPolicyDiagnosticsDesc')}>{body}</SettingsPanel>
  );
};

export default ToolPolicyDiagnosticsPanel;
