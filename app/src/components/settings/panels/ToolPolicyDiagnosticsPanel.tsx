import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import Card from '../../ui/Card';
import { SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

type SecurityPolicyInfo = {
  autonomy: string;
  workspace_only: boolean;
  allowed_commands: string[];
  max_actions_per_hour: number;
  require_approval_for_medium_risk: boolean;
  block_high_risk_commands: boolean;
};

const unwrapPolicy = (raw: unknown): SecurityPolicyInfo => {
  if (raw && typeof raw === 'object' && 'result' in raw) {
    return (raw as { result: SecurityPolicyInfo }).result;
  }
  return raw as SecurityPolicyInfo;
};

const isMethodNotBuilt = (message: string) =>
  /method (not found|not registered)|unknown method|security module.*(off|not built)/i.test(
    message
  );

const ToolPolicyDiagnosticsPanel = () => {
  const { t } = useT();
  const [status, setStatus] = useState<
    | { kind: 'loading' }
    | { kind: 'ready'; policy: SecurityPolicyInfo }
    | { kind: 'unavailable'; message: string }
    | { kind: 'not-built' }
  >({ kind: 'loading' });

  useEffect(() => {
    let cancelled = false;
    const loadPolicy = async () => {
      try {
        const raw = await callCoreRpc<unknown>({
          method: 'openhuman.security_policy_info',
          params: {},
          timeoutMs: 10_000,
        });
        const policy = unwrapPolicy(raw);
        if (!cancelled) setStatus({ kind: 'ready', policy });
      } catch (error) {
        if (cancelled) return;
        const message = error instanceof Error ? error.message : String(error);
        setStatus(
          isMethodNotBuilt(message) ? { kind: 'not-built' } : { kind: 'unavailable', message }
        );
      }
    };
    void loadPolicy();
    return () => {
      cancelled = true;
    };
  }, []);

  let body;
  if (status.kind === 'loading') {
    body = (
      <div className="px-4 py-3 text-sm text-content-muted" data-testid="policy-info-loading">
        {t('devOptions.toolPolicyDiagnostics.loading')}
      </div>
    );
  } else if (status.kind === 'unavailable' || status.kind === 'not-built') {
    body = (
      <div className="px-4 py-3" data-testid={`policy-info-${status.kind}`}>
        <div className="mb-1 text-sm font-semibold text-content">
          {t(
            status.kind === 'not-built'
              ? 'devOptions.toolPolicyDiagnostics.notBuilt'
              : 'devOptions.toolPolicyDiagnostics.unavailable'
          )}
        </div>
        {status.kind === 'unavailable' && (
          <SettingsStatusLine saving={false} error={status.message} savingLabel="" />
        )}
      </div>
    );
  } else {
    const { policy } = status;
    const fields: [string, string][] = [
      [t('devOptions.toolPolicyDiagnostics.policy.autonomy'), policy.autonomy],
      [t('devOptions.toolPolicyDiagnostics.policy.workspaceOnly'), String(policy.workspace_only)],
      [
        t('devOptions.toolPolicyDiagnostics.policy.maxActionsPerHour'),
        String(policy.max_actions_per_hour),
      ],
      [
        t('devOptions.toolPolicyDiagnostics.policy.approvalMediumRisk'),
        String(policy.require_approval_for_medium_risk),
      ],
      [
        t('devOptions.toolPolicyDiagnostics.policy.blockHighRisk'),
        String(policy.block_high_risk_commands),
      ],
    ];
    body = (
      <div className="space-y-4" data-testid="policy-info-ready">
        <Card title={t('devOptions.toolPolicyDiagnostics.policy.title')} divided={false}>
          <dl className="divide-y divide-line-subtle px-4">
            {fields.map(([label, value]) => (
              <div key={label} className="flex items-center justify-between gap-3 py-2.5 text-sm">
                <dt className="text-content-muted">{label}</dt>
                <dd className="font-mono text-content">{value}</dd>
              </div>
            ))}
          </dl>
        </Card>
        <Card title={t('devOptions.toolPolicyDiagnostics.policy.allowedCommands')} divided={false}>
          {policy.allowed_commands.length ? (
            <ul className="flex flex-wrap gap-2 p-4">
              {policy.allowed_commands.map(command => (
                <li key={command} className="rounded bg-surface-muted px-2 py-1 font-mono text-xs">
                  {command}
                </li>
              ))}
            </ul>
          ) : (
            <p className="p-4 text-sm text-content-muted">
              {t('devOptions.toolPolicyDiagnostics.policy.noAllowedCommands')}
            </p>
          )}
        </Card>
      </div>
    );
  }

  return (
    <SettingsPanel description={t('devOptions.toolPolicyDiagnosticsDesc')}>{body}</SettingsPanel>
  );
};

export default ToolPolicyDiagnosticsPanel;
