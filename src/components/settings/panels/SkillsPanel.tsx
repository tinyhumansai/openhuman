import { useEffect, useMemo, useState } from 'react';

import {
  type IntegrationCategory,
  type IntegrationInfo,
  openhumanGetConfig,
  openhumanGetRuntimeFlags,
  openhumanListIntegrations,
  openhumanSetBrowserAllowAll,
  openhumanUpdateBrowserSettings,
  runtimeDisableSkill,
  runtimeEnableSkill,
  runtimeIsSkillEnabled,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const CATEGORY_LABELS: Record<IntegrationCategory, string> = {
  Chat: 'Chat',
  AiModel: 'AI Models',
  Productivity: 'Productivity',
  MusicAudio: 'Music & Audio',
  SmartHome: 'Smart Home',
  ToolsAutomation: 'Tools & Automation',
  MediaCreative: 'Media & Creative',
  Social: 'Social',
  Platform: 'Platform',
};

const SkillsPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const [loading, setLoading] = useState(true);
  const [integrations, setIntegrations] = useState<IntegrationInfo[]>([]);
  const [enabledMap, setEnabledMap] = useState<Record<string, boolean>>({});
  const [toggleBusy, setToggleBusy] = useState<Record<string, boolean>>({});
  const [browserAllowAll, setBrowserAllowAll] = useState<boolean>(false);
  const [browserAllowAllBusy, setBrowserAllowAllBusy] = useState<boolean>(false);
  const [error, setError] = useState<string>('');

  useEffect(() => {
    const loadIntegrations = async () => {
      try {
        const response = await openhumanListIntegrations();
        setIntegrations(response.result);
        const configResponse = await openhumanGetConfig();
        const config = configResponse.result.config as Record<string, unknown>;
        const browserConfig = (config.browser as Record<string, unknown>) ?? {};
        const runtimeFlags = await openhumanGetRuntimeFlags();
        setBrowserAllowAll(runtimeFlags.result.browser_allow_all);
        const entries = await Promise.all(
          response.result.map(async integration => {
            const skillId = integrationSkillId(integration);
            try {
              const enabled =
                integration.name === 'Browser'
                  ? ((browserConfig.enabled as boolean) ?? false)
                  : await runtimeIsSkillEnabled(skillId);
              return [integration.name, enabled] as const;
            } catch {
              return [integration.name, false] as const;
            }
          })
        );
        setEnabledMap(Object.fromEntries(entries));
      } catch (error) {
        console.warn('Could not load integrations from OpenHuman:', error);
        const message = error instanceof Error ? error.message : String(error);
        setError(message);
      } finally {
        setLoading(false);
      }
    };

    loadIntegrations();
  }, []);

  const groupedIntegrations = useMemo(() => {
    return integrations.reduce<Record<IntegrationCategory, IntegrationInfo[]>>(
      (acc, integration) => {
        const category = integration.category;
        if (!acc[category]) {
          acc[category] = [];
        }
        acc[category].push(integration);
        return acc;
      },
      {} as Record<IntegrationCategory, IntegrationInfo[]>
    );
  }, [integrations]);

  return (
    <div className="h-full flex flex-col">
      <SettingsHeader title="Skills" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto px-6 pb-10 space-y-6">
        <section className="rounded-xl border border-stone-800/60 bg-black/40 p-4 space-y-3">
          <div>
            <h3 className="text-lg font-semibold text-white">Browser Access</h3>
            <p className="text-xs text-stone-400">
              Allow the browser tool to visit any public domain (private and file URLs are still
              blocked).
            </p>
          </div>
          <label className="flex items-center gap-3 text-sm text-stone-300">
            <input
              type="checkbox"
              className="checkbox checkbox-primary"
              checked={browserAllowAll}
              disabled={browserAllowAllBusy}
              onChange={async event => {
                const next = event.target.checked;
                setBrowserAllowAllBusy(true);
                try {
                  const response = await openhumanSetBrowserAllowAll(next);
                  setBrowserAllowAll(response.result.browser_allow_all);
                } catch (err) {
                  const message = err instanceof Error ? err.message : String(err);
                  setError(message);
                } finally {
                  setBrowserAllowAllBusy(false);
                }
              }}
            />
            {browserAllowAllBusy
              ? 'Saving…'
              : browserAllowAll
                ? 'Allow all domains'
                : 'Restrict to allowlist'}
          </label>
        </section>

        {error && (
          <div className="rounded-lg border border-red-500/40 bg-red-500/10 px-4 py-3 text-sm text-red-200">
            {error}
          </div>
        )}
        <div className="rounded-xl border border-stone-800/60 bg-black/40">
          {loading && <div className="p-4 text-sm text-stone-400">Loading integrations...</div>}
          {!loading && integrations.length === 0 && (
            <div className="p-4 text-sm text-stone-400">
              No integrations registered in OpenHuman.
            </div>
          )}
          {!loading &&
            Object.entries(groupedIntegrations).map(([category, items]) => (
              <div key={category} className="border-b border-stone-800/60 last:border-0">
                <div className="px-4 pt-4 pb-2 text-xs uppercase tracking-wide text-stone-500">
                  {CATEGORY_LABELS[category as IntegrationCategory] ?? category}
                </div>
                <div>
                  {items.map((integration, index) => (
                    <IntegrationRow
                      key={integration.name}
                      integration={integration}
                      isLast={index === items.length - 1}
                      enabled={enabledMap[integration.name] ?? false}
                      busy={toggleBusy[integration.name] ?? false}
                      toggleable={isIntegrationToggleable(integration)}
                      onToggle={async nextEnabled => {
                        const key = integration.name;
                        setToggleBusy(prev => ({ ...prev, [key]: true }));
                        try {
                          if (integration.name === 'Browser') {
                            await openhumanUpdateBrowserSettings({ enabled: nextEnabled });
                          } else {
                            const skillId = integrationSkillId(integration);
                            if (nextEnabled) {
                              await runtimeEnableSkill(skillId);
                            } else {
                              await runtimeDisableSkill(skillId);
                            }
                          }
                          setEnabledMap(prev => ({ ...prev, [key]: nextEnabled }));
                        } catch (err) {
                          const message = err instanceof Error ? err.message : String(err);
                          setError(message);
                        } finally {
                          setToggleBusy(prev => ({ ...prev, [key]: false }));
                        }
                      }}
                    />
                  ))}
                </div>
              </div>
            ))}
        </div>
      </div>
    </div>
  );
};

function IntegrationRow({
  integration,
  isLast,
  enabled,
  busy,
  toggleable,
  onToggle,
}: {
  integration: IntegrationInfo;
  isLast: boolean;
  enabled: boolean;
  busy: boolean;
  toggleable: boolean;
  onToggle: (nextEnabled: boolean) => void;
}) {
  const statusStyle =
    integration.status === 'Active'
      ? 'bg-sage-500/20 text-sage-300 border-sage-500/30'
      : integration.status === 'Available'
        ? 'bg-amber-500/15 text-amber-300 border-amber-500/30'
        : 'bg-stone-500/20 text-stone-300 border-stone-500/30';

  return (
    <div
      className={`flex items-center justify-between gap-4 p-4 ${
        isLast ? '' : 'border-b border-stone-800/60'
      }`}>
      <div className="flex items-center gap-3 text-left flex-1 min-w-0">
        <div className="w-6 h-6 flex items-center justify-center text-white/70">
          <span className="text-xs font-semibold uppercase">{integration.name.slice(0, 2)}</span>
        </div>
        <div className="min-w-0">
          <div className="text-sm font-semibold text-white truncate">{integration.name}</div>
          <div className="text-xs text-stone-400 line-clamp-2">{integration.description}</div>
          {integration.setup_hints.length > 0 && (
            <div className="mt-1 text-[11px] text-stone-500">{integration.setup_hints[0]}</div>
          )}
        </div>
      </div>

      <div className="flex items-center gap-3">
        <span
          className={`px-2 py-1 text-[11px] font-semibold uppercase border rounded-full ${statusStyle}`}>
          {integration.status}
        </span>
        <label className="flex items-center gap-2 text-xs text-stone-300">
          <input
            type="checkbox"
            className="checkbox checkbox-primary"
            checked={enabled}
            disabled={busy || !toggleable}
            onChange={event => onToggle(event.target.checked)}
          />
          {busy ? 'Saving…' : enabled ? 'Enabled' : toggleable ? 'Disabled' : 'Managed'}
        </label>
      </div>
    </div>
  );
}

export default SkillsPanel;

function integrationSkillId(integration: IntegrationInfo): string {
  return integration.name
    .trim()
    .toLowerCase()
    .replace(/\\s+/g, '-')
    .replace(/[^a-z0-9\\-_]/g, '');
}

function isIntegrationToggleable(integration: IntegrationInfo): boolean {
  if (integration.name === 'Browser') {
    return true;
  }
  return integration.category === 'Chat' || integration.category === 'ToolsAutomation';
}
