/**
 * The TinyComputer module at a glance, shared by the Desktop, Browser and
 * Models sub-tabs: its lifecycle state and pinned version, which decision
 * model and routes the host configured, and — after a check — what the
 * module itself reports through `Describe`.
 */
import { RefreshCw } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import { Alert, AlertDescription } from '../../ui/Alert';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import Card from '../../ui/Card';

type ModuleState = 'available' | 'loading' | 'ready' | 'failed' | 'unsupported' | 'disabled';
type Route = 'hosted' | 'direct_openrouter' | 'open_jev' | 'sage' | 'unavailable';

export interface ComputerStatus {
  module: { id: string; version: string; state: ModuleState; detail?: string | null } | null;
  decision_model: 'jev' | 'open_jev' | 'sage';
  decision_route: Route;
  planner_route: Route;
  capabilities?: {
    contract_version: [number, number];
    compatible: boolean;
    jev_configured: boolean;
    planner_configured: boolean;
    rescue_configured: boolean;
    surfaces: { kind: 'desktop' | 'browser'; available: boolean; reason?: string }[];
  };
  error?: string;
}

const stateVariant = (state?: ModuleState) =>
  state === 'ready'
    ? ('success' as const)
    : state === 'failed' || state === 'unsupported'
      ? ('danger' as const)
      : state === 'disabled'
        ? ('warning' as const)
        : ('neutral' as const);

export default function ComputerStatusCard({ refreshKey = 0 }: { refreshKey?: number } = {}) {
  const { t } = useT();
  const [status, setStatus] = useState<ComputerStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (check: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(
        await callCoreRpc<ComputerStatus>({
          method: 'openhuman.modules_computer_status',
          params: { load: check },
        })
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void Promise.resolve().then(() => load(false));
  }, [load, refreshKey]);

  const model = (value?: string) =>
    value === 'open_jev'
      ? t('computer.models.openJev')
      : value === 'sage'
        ? t('computer.models.sage')
        : t('computer.models.jev');
  const route = (value?: Route) =>
    value === 'hosted'
      ? t('connections.browser.routeHosted')
      : value === 'direct_openrouter'
        ? t('connections.browser.routeDirect')
        : value === 'open_jev' || value === 'sage'
          ? t('computer.status.ownKey')
          : t('computer.status.noCredential');
  const flag = (label: string, on: boolean) => (
    <Badge key={label} variant={on ? 'success' : 'warning'}>
      {label}: {on ? t('computer.status.configured') : t('computer.status.missing')}
    </Badge>
  );

  const capabilities = status?.capabilities;
  return (
    <Card title={t('computer.status.title')} padded divided={false}>
      <div className="space-y-3 text-sm" data-testid="computer-status">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <Badge
              variant={stateVariant(status?.module?.state)}
              data-testid="computer-module-state">
              {status?.module
                ? t(`computer.status.state.${status.module.state}`)
                : t('connections.browser.unknown')}
            </Badge>
            {status?.module && <span className="text-content-muted">v{status.module.version}</span>}
          </div>
          <Button
            variant="secondary"
            size="sm"
            disabled={busy}
            leadingIcon={<RefreshCw className="h-3.5 w-3.5" aria-hidden />}
            onClick={() => void load(true)}>
            {t('computer.status.check')}
          </Button>
        </div>
        {status && (
          <dl className="grid gap-2 sm:grid-cols-2">
            <div>
              <dt className="text-content-muted">{t('computer.models.decisionModel')}</dt>
              <dd className="font-medium text-content" data-testid="computer-decision">
                {model(status.decision_model)} · {route(status.decision_route)}
              </dd>
            </div>
            <div>
              <dt className="text-content-muted">{t('computer.models.rescueTitle')}</dt>
              <dd className="font-medium text-content" data-testid="computer-planner">
                {route(status.planner_route)}
              </dd>
            </div>
          </dl>
        )}
        {capabilities && (
          <div className="flex flex-wrap gap-2" data-testid="computer-capabilities">
            {!capabilities.compatible && (
              <Badge variant="danger">{t('computer.status.incompatible')}</Badge>
            )}
            {flag(t('computer.models.decisionTitle'), capabilities.jev_configured)}
            {flag(t('computer.models.plannerModel'), capabilities.planner_configured)}
            {flag(t('computer.models.rescueModel'), capabilities.rescue_configured)}
            {capabilities.surfaces.map(surface => (
              <Badge
                key={surface.kind}
                variant={surface.available ? 'success' : 'warning'}
                title={surface.reason}>
                {t(`computer.tabs.${surface.kind}`)}:{' '}
                {surface.available
                  ? t('computer.status.available')
                  : t('computer.status.unavailable')}
              </Badge>
            ))}
          </div>
        )}
        {status?.module?.detail && <p className="text-content-muted">{status.module.detail}</p>}
        {(error || status?.error) && (
          <Alert variant="destructive" density="compact">
            <AlertDescription>{error ?? status?.error}</AlertDescription>
          </Alert>
        )}
      </div>
    </Card>
  );
}
