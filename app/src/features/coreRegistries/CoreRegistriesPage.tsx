import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { RegistryBridgeErrorMeta } from '../../services/api/coreRegistriesClient';
import RegistryCollectionPane, { type RegistryCollectionPaneItem } from './RegistryCollectionPane';
import RegistryDetailDrawer from './RegistryDetailDrawer';
import RegistryDetailPane from './RegistryDetailPane';
import { REGISTRY_TABS, type RegistryTab } from './types';
import { useRegistryInspection } from './useRegistryInspection';

type TranslateFn = (key: string, fallback?: string) => string;

function formatLiteral(value: string): string {
  return value
    .split(/[_-]/)
    .filter(Boolean)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

function shortFingerprint(value: string): string {
  return value.slice(0, 12);
}

function fingerprintLabel(
  t: TranslateFn,
  family: 'config' | 'definition' | 'type' | 'binding',
  value: string
): string {
  return t(`registries.items.fingerprint.${family}`).replace(
    '{fingerprint}',
    shortFingerprint(value)
  );
}

function tabLabel(t: TranslateFn, tab: RegistryTab): string {
  return t(`registries.tab.${tab}`);
}

function summaryStateLabel(t: TranslateFn, summaryState: string): string {
  return t(`registries.summaryState.${summaryState}`);
}

function tabSummary(t: TranslateFn, tab: RegistryTab, summaryState: string) {
  const label = summaryStateLabel(t, summaryState);
  if (summaryState === 'fresh') {
    return `${tabLabel(t, tab)} · ${t('registries.summaryState.observed')}`;
  }
  return `${tabLabel(t, tab)} · ${label}`;
}

function describeBlocker(t: TranslateFn, error: RegistryBridgeErrorMeta) {
  if (error.kind === 'YouPetConfigMissing' || error.kind === 'YouPetConfigInvalid') {
    return error.kind === 'YouPetConfigMissing'
      ? {
          title: t('registries.blocker.configRequiredTitle'),
          description: t('registries.blocker.configMissingDescription'),
        }
      : {
          title: t('registries.blocker.configInvalidTitle'),
          description: t('registries.blocker.configInvalidDescription'),
        };
  }

  if (error.kind === 'YouPetCoreHttpError' && error.httpStatus === 401) {
    return {
      title: t('registries.blocker.authRequiredTitle'),
      description: t('registries.blocker.authRequiredDescription'),
    };
  }

  if (
    error.kind === 'YouPetCoreHttpError' &&
    error.httpStatus === 403 &&
    error.coreCode === 'forbidden_actor'
  ) {
    return {
      title: t('registries.blocker.forbiddenTitle'),
      description: t('registries.blocker.forbiddenDescription'),
    };
  }

  if (
    error.kind === 'YouPetCoreHttpError' &&
    error.httpStatus === 503 &&
    error.coreCode === 'kernel_tenant_unavailable'
  ) {
    return {
      title: t('registries.blocker.tenantUnavailableTitle'),
      description: t('registries.blocker.tenantUnavailableDescription'),
    };
  }

  if (
    error.kind === 'YouPetCoreHttpError' &&
    error.httpStatus === 503 &&
    error.coreCode === 'kernel_tenant_invariant_violation'
  ) {
    return {
      title: t('registries.blocker.tenantInvariantTitle'),
      description: t('registries.blocker.tenantInvariantDescription'),
    };
  }

  return {
    title: t('registries.blocker.configRequiredTitle'),
    description: t('registries.blocker.genericDescription'),
  };
}

function hasMore(nextCursor: string | null) {
  return typeof nextCursor === 'string' && nextCursor.length > 0;
}

function isRetryDisabled(retryDisabledUntil: number | null | undefined): boolean {
  return typeof retryDisabledUntil === 'number' && retryDisabledUntil > Date.now();
}

const MAX_BROWSER_TIMEOUT_MS = 2_147_483_647;

function useMinWidth(query: string): boolean {
  const getMatches = () =>
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia(query).matches
      : false;
  const [matches, setMatches] = useState(getMatches);

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      return;
    }

    const mediaQuery = window.matchMedia(query);
    const update = (event?: MediaQueryListEvent) =>
      setMatches(event?.matches ?? mediaQuery.matches);

    update();
    mediaQuery.addEventListener?.('change', update);
    mediaQuery.addListener?.(update);
    return () => {
      mediaQuery.removeEventListener?.('change', update);
      mediaQuery.removeListener?.(update);
    };
  }, [query]);

  return matches;
}

function tabId(tab: RegistryTab): string {
  return `registry-tab-${tab}`;
}

function tabPanelId(tab: RegistryTab): string {
  return `registry-panel-${tab}`;
}

export default function CoreRegistriesPage() {
  const { t } = useT();
  const { state, setTab, refreshActiveTab, loadMoreCollection, openDetail, retryCollection } =
    useRegistryInspection();
  const activeTab = state.urlState.tab;
  const detailState = state.tabs[activeTab].detail;
  const isWideLayout = useMinWidth('(min-width: 1280px)');
  const tabRefs = useRef<Record<RegistryTab, HTMLButtonElement | null>>({
    agents: null,
    tools: null,
    connectors: null,
  });
  const blocker = state.surfaceError ? describeBlocker(t, state.surfaceError) : null;
  const [retryClockTick, setRetryClockTick] = useState(0);

  const activeRetryDisabledUntilValues = useMemo(() => {
    if (activeTab === 'agents') {
      return [state.tabs.agents.collections.agents.retryDisabledUntil];
    }

    if (activeTab === 'tools') {
      return [
        state.tabs.tools.collections.toolDefinitions.retryDisabledUntil,
        state.tabs.tools.collections.toolEnablements.retryDisabledUntil,
      ];
    }

    return [
      state.tabs.connectors.collections.connectorTypes.retryDisabledUntil,
      state.tabs.connectors.collections.connectorBindings.retryDisabledUntil,
    ];
  }, [activeTab, state.tabs]);

  const activeTabRetryDisabled = activeRetryDisabledUntilValues.some(isRetryDisabled);

  useEffect(() => {
    const now = Date.now();
    const nextRetryWakeAt = activeRetryDisabledUntilValues
      .filter((value): value is number => typeof value === 'number' && value > now)
      .sort((left, right) => left - right)[0];

    if (!nextRetryWakeAt) {
      return;
    }

    const timer = window.setTimeout(
      () => {
        setRetryClockTick(current => current + 1);
      },
      Math.min(nextRetryWakeAt - now, MAX_BROWSER_TIMEOUT_MS)
    );

    return () => window.clearTimeout(timer);
  }, [activeRetryDisabledUntilValues, retryClockTick]);

  const agentItems = useMemo<RegistryCollectionPaneItem[]>(
    () =>
      state.tabs.agents.collections.agents.items.map(agent => ({
        id: agent.id,
        title: agent.agentKey,
        subtitle: `v${agent.version} · ${formatLiteral(agent.ownerActorType)} · ${agent.ownerActorId}`,
        meta: [
          t('registries.items.created').replace(
            '{date}',
            new Date(agent.createdAt).toLocaleDateString()
          ),
        ],
        statusLabel: formatLiteral(agent.lifecycleState),
        fingerprintLabel: fingerprintLabel(t, 'config', agent.configurationFingerprint),
        onSelect: () =>
          void openDetail({ kind: 'agent', key: agent.agentKey, version: agent.version }),
      })),
    [openDetail, state.tabs.agents.collections.agents.items, t]
  );

  const toolItems = useMemo<RegistryCollectionPaneItem[]>(
    () =>
      state.tabs.tools.collections.toolDefinitions.items.map(definition => {
        const enablement = state.tabs.tools.collections.toolEnablements.items.find(
          item => item.toolKey === definition.toolKey && item.version === definition.version
        );
        const statusLabel = enablement
          ? enablement.lifecycleState === 'enabled'
            ? t('common.enabled')
            : t('common.disabled')
          : t('registries.items.status.noTenantEnablement');

        return {
          id: `${definition.toolKey}:${definition.version}`,
          title: definition.displayName,
          subtitle: `${definition.toolKey} v${definition.version}`,
          meta: [formatLiteral(definition.toolEffectClass)],
          statusLabel,
          fingerprintLabel: fingerprintLabel(t, 'definition', definition.definitionFingerprint),
          onSelect: () =>
            void openDetail({
              kind: 'tool-definition',
              key: definition.toolKey,
              version: definition.version,
            }),
        };
      }),
    [
      openDetail,
      state.tabs.tools.collections.toolDefinitions.items,
      state.tabs.tools.collections.toolEnablements.items,
      t,
    ]
  );

  const enablementItems = useMemo<RegistryCollectionPaneItem[]>(
    () =>
      state.tabs.tools.collections.toolEnablements.items.map(enablement => ({
        id: `${enablement.toolKey}:${enablement.version}`,
        title: enablement.toolKey,
        subtitle: `v${enablement.version} · generation ${enablement.generation}`,
        meta: [
          enablement.auditMode
            ? formatLiteral(enablement.auditMode)
            : t('registries.items.meta.noAuditMode'),
          enablement.approvalRequired
            ? t('registries.items.meta.approvalRequired')
            : t('registries.items.meta.noApprovalGate'),
        ],
        statusLabel: formatLiteral(enablement.lifecycleState),
        onSelect: () =>
          void openDetail({
            kind: 'tool-enablement',
            key: enablement.toolKey,
            version: enablement.version,
          }),
      })),
    [openDetail, state.tabs.tools.collections.toolEnablements.items, t]
  );

  const connectorTypeItems = useMemo<RegistryCollectionPaneItem[]>(
    () =>
      state.tabs.connectors.collections.connectorTypes.items.map(connectorType => ({
        id: `${connectorType.connectorKey}:${connectorType.version}`,
        title: connectorType.connectorKey,
        subtitle: `v${connectorType.version} · ${connectorType.sourceType}`,
        meta: connectorType.capabilities,
        statusLabel: formatLiteral(connectorType.lifecycleState),
        fingerprintLabel: fingerprintLabel(t, 'type', connectorType.connectorTypeFingerprint),
        onSelect: () =>
          void openDetail({
            kind: 'connector-type',
            key: connectorType.connectorKey,
            version: connectorType.version,
          }),
      })),
    [openDetail, state.tabs.connectors.collections.connectorTypes.items, t]
  );

  const connectorBindingItems = useMemo<RegistryCollectionPaneItem[]>(
    () =>
      state.tabs.connectors.collections.connectorBindings.items.map(binding => ({
        id: `${binding.bindingKey}:${binding.version}`,
        title: binding.bindingKey,
        subtitle: `v${binding.version} · ${binding.connectorTypeKey} v${binding.connectorTypeVersion}`,
        meta: binding.enabledCapabilities,
        statusLabel: formatLiteral(binding.lifecycleState),
        fingerprintLabel: fingerprintLabel(t, 'binding', binding.bindingFingerprint),
        onSelect: () =>
          void openDetail({
            kind: 'connector-binding',
            key: binding.bindingKey,
            version: binding.version,
          }),
      })),
    [openDetail, state.tabs.connectors.collections.connectorBindings.items, t]
  );

  const selectedDetailTitle =
    detailState.kind === 'loaded' || detailState.kind === 'loading'
      ? `${detailState.detail.key} v${detailState.detail.version}`
      : t('registries.drawer.title');

  const closeDetail = () => {
    void setTab(activeTab);
  };

  const liveMessage = blocker
    ? t('registries.live.blocked')
        .replace('{title}', blocker.title)
        .replace('{description}', blocker.description)
    : detailState.kind === 'loaded'
      ? t('registries.live.loaded')
          .replace('{tab}', tabLabel(t, activeTab))
          .replace('{key}', detailState.detail.key)
          .replace('{version}', String(detailState.detail.version))
      : detailState.kind === 'loading'
        ? t('registries.live.loading')
            .replace('{tab}', tabLabel(t, activeTab))
            .replace('{key}', detailState.detail.key)
            .replace('{version}', String(detailState.detail.version))
        : t('registries.live.idle').replace('{tab}', tabLabel(t, activeTab));

  const handleTabKeyDown = async (event: KeyboardEvent<HTMLButtonElement>, tab: RegistryTab) => {
    const currentIndex = REGISTRY_TABS.indexOf(tab);
    if (currentIndex === -1) {
      return;
    }

    let nextIndex: number | null = null;
    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        nextIndex = (currentIndex + 1) % REGISTRY_TABS.length;
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        nextIndex = (currentIndex - 1 + REGISTRY_TABS.length) % REGISTRY_TABS.length;
        break;
      case 'Home':
        nextIndex = 0;
        break;
      case 'End':
        nextIndex = REGISTRY_TABS.length - 1;
        break;
      case 'Enter':
      case ' ':
        event.preventDefault();
        await setTab(tab);
        return;
      default:
        return;
    }

    event.preventDefault();
    const nextTab = REGISTRY_TABS[nextIndex];
    if (!nextTab) {
      return;
    }
    tabRefs.current[nextTab]?.focus();
    await setTab(nextTab);
  };

  return (
    <div className="min-h-full bg-stone-50 px-4 py-8 dark:bg-neutral-950">
      <div className="mx-auto max-w-7xl space-y-6">
        <div role="status" aria-live="polite" aria-atomic="true" className="sr-only">
          {liveMessage}
        </div>

        <header className="rounded-[28px] border border-stone-200 bg-white px-6 py-6 shadow-soft dark:border-neutral-800 dark:bg-neutral-900">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="max-w-3xl">
              <p className="text-[11px] font-semibold uppercase tracking-[0.3em] text-stone-500 dark:text-neutral-400">
                {t('registries.page.eyebrow')}
              </p>
              <h1 className="mt-2 text-3xl font-semibold text-stone-900 dark:text-neutral-100">
                {t('registries.page.title')}
              </h1>
              <p className="mt-2 text-sm leading-6 text-stone-600 dark:text-neutral-300">
                {t('registries.page.description')} {t('registries.page.readOnly')}
              </p>
            </div>

            <button
              type="button"
              disabled={activeTabRetryDisabled}
              onClick={() => {
                void refreshActiveTab();
              }}
              className="inline-flex items-center rounded-2xl border border-stone-200 px-4 py-2 text-sm font-medium text-stone-700 transition hover:bg-stone-100 disabled:cursor-not-allowed disabled:opacity-50 dark:border-neutral-700 dark:text-neutral-200 dark:hover:bg-neutral-800">
              {t('common.refresh')}
            </button>
          </div>
        </header>

        {state.surfaceError ? (
          <section className="rounded-[28px] border border-amber-200 bg-amber-50 px-6 py-6 text-amber-900 shadow-soft">
            <h2 className="text-lg font-semibold">{blocker?.title}</h2>
            <p className="mt-2 text-sm">{blocker?.description}</p>
            <p className="mt-2 text-sm">{t('registries.blocker.readOnly')}</p>
            <p className="mt-1 text-sm">{t('registries.blocker.fixFlow')}</p>
            <button
              type="button"
              onClick={() => {
                void refreshActiveTab();
              }}
              className="mt-4 inline-flex items-center rounded-2xl bg-amber-500 px-4 py-2 text-sm font-medium text-white transition hover:bg-amber-600">
              {t('registries.page.retry')}
            </button>
          </section>
        ) : (
          <>
            <section className="rounded-[28px] border border-stone-200 bg-white p-4 shadow-soft dark:border-neutral-800 dark:bg-neutral-900">
              <div
                role="tablist"
                aria-label={t('registries.page.tablistAria')}
                className="flex flex-wrap gap-3">
                {REGISTRY_TABS.map(tab => {
                  const selected = tab === activeTab;
                  return (
                    <button
                      key={tab}
                      id={tabId(tab)}
                      ref={node => {
                        tabRefs.current[tab] = node;
                      }}
                      type="button"
                      role="tab"
                      aria-controls={tabPanelId(tab)}
                      aria-selected={selected}
                      tabIndex={selected ? 0 : -1}
                      onClick={() => {
                        void setTab(tab);
                      }}
                      onKeyDown={event => {
                        void handleTabKeyDown(event, tab);
                      }}
                      className={`inline-flex items-center rounded-2xl px-4 py-2 text-sm font-medium transition ${
                        selected
                          ? 'bg-stone-900 text-white dark:bg-white dark:text-neutral-900'
                          : 'border border-stone-200 text-stone-700 hover:bg-stone-100 dark:border-neutral-700 dark:text-neutral-200 dark:hover:bg-neutral-800'
                      }`}>
                      {tabLabel(t, tab)}
                    </button>
                  );
                })}
              </div>

              <div className="mt-4 flex flex-wrap gap-3">
                {REGISTRY_TABS.map(tab => (
                  <span
                    key={`${tab}-summary`}
                    className="inline-flex items-center rounded-full border border-stone-200 bg-stone-50 px-3 py-1 text-[11px] font-medium text-stone-700 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                    {tabSummary(t, tab, state.tabs[tab].summaryState)}
                  </span>
                ))}
              </div>
            </section>

            <div className="grid gap-6 xl:grid-cols-[minmax(0,1fr)_320px]">
              <section
                id={tabPanelId(activeTab)}
                role="tabpanel"
                aria-labelledby={tabId(activeTab)}
                className="space-y-6"
                tabIndex={0}>
                {activeTab === 'agents' ? (
                  <RegistryCollectionPane
                    title={t('registries.collections.agents.title')}
                    description={t('registries.collections.agents.description')}
                    observation={state.tabs.agents.collections.agents.observation}
                    items={agentItems}
                    hasMore={hasMore(state.tabs.agents.collections.agents.nextCursor)}
                    loadMoreLabel={t('registries.collections.agents.loadMore')}
                    onLoadMore={() => {
                      void loadMoreCollection('agents');
                    }}
                    onRetry={() => {
                      void retryCollection('agents');
                    }}
                    retryDisabled={isRetryDisabled(
                      state.tabs.agents.collections.agents.retryDisabledUntil
                    )}
                    loadMoreDisabled={isRetryDisabled(
                      state.tabs.agents.collections.agents.retryDisabledUntil
                    )}
                  />
                ) : null}

                {activeTab === 'tools' ? (
                  <div className="grid gap-6 xl:grid-cols-2">
                    <RegistryCollectionPane
                      title={t('registries.collections.toolDefinitions.title')}
                      description={t('registries.collections.toolDefinitions.description')}
                      observation={state.tabs.tools.collections.toolDefinitions.observation}
                      items={toolItems}
                      hasMore={hasMore(state.tabs.tools.collections.toolDefinitions.nextCursor)}
                      loadMoreLabel={t('registries.collections.toolDefinitions.loadMore')}
                      onLoadMore={() => {
                        void loadMoreCollection('toolDefinitions');
                      }}
                      onRetry={() => {
                        void retryCollection('toolDefinitions');
                      }}
                      retryDisabled={isRetryDisabled(
                        state.tabs.tools.collections.toolDefinitions.retryDisabledUntil
                      )}
                      loadMoreDisabled={isRetryDisabled(
                        state.tabs.tools.collections.toolDefinitions.retryDisabledUntil
                      )}
                    />
                    <RegistryCollectionPane
                      title={t('registries.collections.toolEnablements.title')}
                      description={t('registries.collections.toolEnablements.description')}
                      observation={state.tabs.tools.collections.toolEnablements.observation}
                      items={enablementItems}
                      onRetry={() => {
                        void retryCollection('toolEnablements');
                      }}
                      retryDisabled={isRetryDisabled(
                        state.tabs.tools.collections.toolEnablements.retryDisabledUntil
                      )}
                    />
                  </div>
                ) : null}

                {activeTab === 'connectors' ? (
                  <div className="grid gap-6 xl:grid-cols-2">
                    <RegistryCollectionPane
                      title={t('registries.collections.connectorTypes.title')}
                      description={t('registries.collections.connectorTypes.description')}
                      observation={state.tabs.connectors.collections.connectorTypes.observation}
                      items={connectorTypeItems}
                      hasMore={hasMore(state.tabs.connectors.collections.connectorTypes.nextCursor)}
                      loadMoreLabel={t('registries.collections.connectorTypes.loadMore')}
                      onLoadMore={() => {
                        void loadMoreCollection('connectorTypes');
                      }}
                      onRetry={() => {
                        void retryCollection('connectorTypes');
                      }}
                      retryDisabled={isRetryDisabled(
                        state.tabs.connectors.collections.connectorTypes.retryDisabledUntil
                      )}
                      loadMoreDisabled={isRetryDisabled(
                        state.tabs.connectors.collections.connectorTypes.retryDisabledUntil
                      )}
                    />
                    <RegistryCollectionPane
                      title={t('registries.collections.connectorBindings.title')}
                      description={t('registries.collections.connectorBindings.description')}
                      observation={state.tabs.connectors.collections.connectorBindings.observation}
                      items={connectorBindingItems}
                      hasMore={hasMore(
                        state.tabs.connectors.collections.connectorBindings.nextCursor
                      )}
                      loadMoreLabel={t('registries.collections.connectorBindings.loadMore')}
                      onLoadMore={() => {
                        void loadMoreCollection('connectorBindings');
                      }}
                      onRetry={() => {
                        void retryCollection('connectorBindings');
                      }}
                      retryDisabled={isRetryDisabled(
                        state.tabs.connectors.collections.connectorBindings.retryDisabledUntil
                      )}
                      loadMoreDisabled={isRetryDisabled(
                        state.tabs.connectors.collections.connectorBindings.retryDisabledUntil
                      )}
                    />
                  </div>
                ) : null}
              </section>

              {isWideLayout ? (
                <aside className="block">
                  <div className="sticky top-8">
                    <RegistryDetailPane
                      activeTab={activeTab}
                      detailState={detailState}
                      state={state}
                      onOpenDetail={openDetail}
                    />
                  </div>
                </aside>
              ) : null}
            </div>
          </>
        )}
      </div>

      {!isWideLayout && detailState.kind !== 'none' ? (
        <RegistryDetailDrawer title={selectedDetailTitle} onClose={closeDetail}>
          <RegistryDetailPane
            activeTab={activeTab}
            detailState={detailState}
            state={state}
            onOpenDetail={openDetail}
          />
        </RegistryDetailDrawer>
      ) : null}
    </div>
  );
}
