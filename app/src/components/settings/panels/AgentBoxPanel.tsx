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
import PanelPage from '../../layout/PanelPage';
import SettingsBackButton from '../components/SettingsBackButton';
import { SettingsStatusLine } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

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

const ROW =
  'px-4 py-3 rounded-lg border border-sage-300 dark:border-sage-500/40 bg-white dark:bg-sage-900/20';

const AgentBoxPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();

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
      return (
        <div className="px-4 py-3 text-sm text-neutral-500 dark:text-neutral-400">
          {t('common.loading')}
        </div>
      );
    }
    if (state.kind === 'error') {
      return (
        <div className="px-4 py-3">
          <div className="text-sm font-semibold text-neutral-800 dark:text-neutral-100 mb-1">
            {t('settings.agentbox.unavailable')}
          </div>
          <SettingsStatusLine saving={false} error={state.message} savingLabel="" />
        </div>
      );
    }

    const s = state.status;
    const modeLabel = s.mode_enabled ? t('common.enabled') : t('common.disabled');

    return (
      <div className="px-4 pt-3 pb-6 flex flex-col gap-3">
        <div className="text-xs text-sage-700 dark:text-sage-300">
          {t('settings.agentbox.intro')}
        </div>

        <div className={ROW}>
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-semibold text-sage-900 dark:text-sage-200">
              {t('settings.agentbox.modeLabel')}
            </span>
            <span
              className={`text-xs font-mono px-2 py-0.5 rounded-full ${
                s.mode_enabled
                  ? 'bg-sage-100 text-sage-800 dark:bg-sage-500/20 dark:text-sage-200'
                  : 'bg-neutral-100 text-neutral-600 dark:bg-neutral-700/40 dark:text-neutral-300'
              }`}>
              {modeLabel}
            </span>
          </div>
        </div>

        <div className={ROW}>
          <div className="text-sm font-semibold text-sage-900 dark:text-sage-200 mb-2">
            {t('settings.agentbox.providerHeading')}
          </div>
          {s.provider_configured && s.provider ? (
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
              <dt className="text-sage-700 dark:text-sage-300">{t('settings.agentbox.slug')}</dt>
              <dd className="font-mono text-sage-900 dark:text-sage-200 break-all">
                {s.provider.slug}
              </dd>
              <dt className="text-sage-700 dark:text-sage-300">{t('settings.agentbox.baseUrl')}</dt>
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

        <button
          type="button"
          onClick={() => void load()}
          className="self-start text-xs font-medium px-3 py-1.5 rounded-md border border-sage-300 dark:border-sage-500/40 text-sage-800 dark:text-sage-200 hover:bg-sage-50 dark:hover:bg-sage-500/10">
          {t('common.refresh')}
        </button>
      </div>
    );
  }, [state, t, load]);

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      description={t('settings.agentbox.desc')}
      leading={<SettingsBackButton onBack={navigateBack} />}>
      {body}
    </PanelPage>
  );
};

export default AgentBoxPanel;
