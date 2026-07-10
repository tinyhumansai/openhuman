// [settings] AgentBox marketplace adapter — read-only status panel.
//
// Surfaces whether the GMI Cloud AgentBox adapter is active and how the GMI
// MaaS provider is wired (slug / base URL / model). Mode and provider are
// configured by environment variables at core startup (OPENHUMAN_AGENTBOX_MODE,
// GMI_MAAS_*), so this panel is intentionally read-only — it reports what the
// running core sees. The API key is never returned by the backend.
import { useCallback, useEffect, useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import Button from '../../ui/Button';
import { SettingsRow, SettingsSection, SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

interface AgentBoxProviderInfo {
  slug: string;
  base_url: string;
  model: string;
}

interface AgentBoxStatus {
  mode_enabled: boolean;
  provider_configured: boolean;
  provider?: AgentBoxProviderInfo | null;
}

type PanelState =
  | { kind: 'loading' }
  | { kind: 'ready'; status: AgentBoxStatus }
  | { kind: 'error'; message: string };

const AgentBoxPanel = () => {
  const { t } = useT();

  const [state, setState] = useState<PanelState>({ kind: 'loading' });

  const load = useCallback(async () => {
    setState({ kind: 'loading' });
    try {
      const status = await callCoreRpc<AgentBoxStatus>({
        method: 'openhuman.agentbox_status',
        params: {},
        timeoutMs: 10_000,
      });
      setState({ kind: 'ready', status });
    } catch (err) {
      setState({ kind: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      if (cancelled) return;
      await load();
    })();
    return () => {
      cancelled = true;
    };
  }, [load]);

  const body = useMemo(() => {
    if (state.kind === 'loading') {
      return <div className="px-4 py-3 text-sm text-content-muted">{t('common.loading')}</div>;
    }
    if (state.kind === 'error') {
      return (
        <div className="px-4 py-3">
          <div className="text-sm font-semibold text-content mb-1">
            {t('settings.agentbox.unavailable')}
          </div>
          <SettingsStatusLine saving={false} error={state.message} savingLabel="" />
        </div>
      );
    }

    const s = state.status;
    const modeLabel = s.mode_enabled ? t('common.enabled') : t('common.disabled');

    return (
      <div className="px-4 pt-3 pb-6 space-y-3">
        <div className="text-xs text-sage-700 dark:text-sage-300">
          {t('settings.agentbox.intro')}
        </div>

        <SettingsSection>
          <SettingsRow
            label={t('settings.agentbox.modeLabel')}
            control={
              <span
                className={`text-xs font-mono px-2 py-0.5 rounded-full ${
                  s.mode_enabled
                    ? 'bg-sage-100 text-sage-800 dark:bg-sage-500/20 dark:text-sage-200'
                    : 'bg-surface-subtle text-content-secondary dark:bg-neutral-700/40'
                }`}>
                {modeLabel}
              </span>
            }
          />
        </SettingsSection>

        <SettingsSection title={t('settings.agentbox.providerHeading')}>
          <div className="px-4 py-3">
            {s.provider_configured && s.provider ? (
              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
                <dt className="text-sage-700 dark:text-sage-300">{t('settings.agentbox.slug')}</dt>
                <dd className="font-mono text-sage-900 dark:text-sage-200 break-all">
                  {s.provider.slug}
                </dd>
                <dt className="text-sage-700 dark:text-sage-300">
                  {t('settings.agentbox.baseUrl')}
                </dt>
                <dd className="font-mono text-sage-900 dark:text-sage-200 break-all">
                  {s.provider.base_url}
                </dd>
                <dt className="text-sage-700 dark:text-sage-300">{t('settings.agentbox.model')}</dt>
                <dd className="font-mono text-sage-900 dark:text-sage-200 break-all">
                  {s.provider.model}
                </dd>
              </dl>
            ) : (
              <div className="text-xs text-sage-700 dark:text-sage-300">
                {t('settings.agentbox.notConfigured')}
              </div>
            )}
          </div>
        </SettingsSection>

        <Button type="button" variant="secondary" size="xs" onClick={() => void load()}>
          {t('common.refresh')}
        </Button>
      </div>
    );
  }, [state, t, load]);

  return <SettingsPanel description={t('settings.agentbox.desc')}>{body}</SettingsPanel>;
};

export default AgentBoxPanel;
