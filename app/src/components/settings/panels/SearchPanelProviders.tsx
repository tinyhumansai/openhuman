/*
 * The Providers tab of the web search settings, in the LLM page's shape:
 * a short "Connected" list of what the agent can use, then an "Add a
 * provider" catalogue of everything that is off.
 *
 * WHY NOT ONE CARD PER PROVIDER. The old tab rendered all nine providers the
 * same way — switch, status badge, a Managed/Own-key segmented control and an
 * inline key editor — so the two providers that were actually on looked like
 * the seven that were not, and the page opened on a column of empty password
 * fields. Splitting on/off answers "what am I using" first. The catalogue then
 * sorts the rest by the one fact that decides the effort: "via TinyHumans"
 * works with a click, "your own key" asks for a key.
 */
import { Plus } from 'lucide-react';
import { useState } from 'react';

import type {
  SearchProviderInfo,
  SearchProviderUpdate,
  SearchSettings,
} from '../../../utils/tauriCommands/config';
import Badge from '../../ui/Badge';
import Card from '../../ui/Card';
import Switch from '../../ui/Switch';
import { ProviderGroup, ProviderListRow, type ProviderRowAction } from './ai/ProviderListRow';
import SearchPanelConnectDialog, { type ConnectMode } from './SearchPanelConnectDialog';
import {
  canUseDirect,
  canUseManaged,
  directNeedsSetup,
  hasBaseUrl,
  isStrandedManaged,
  rolesSummary,
  SearchProviderSwatch,
  STATUS_VARIANT,
  statusLabel,
  type Translate,
  withProvider,
} from './searchPanelShared';

const tileClass =
  'group flex w-full min-w-0 items-center gap-3 rounded-xl border border-line bg-surface px-3 py-2.5 text-left transition-colors enabled:hover:border-line-strong enabled:hover:bg-surface-hover focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25 disabled:cursor-not-allowed disabled:opacity-60';

interface DialogState {
  id: string;
  mode: ConnectMode;
  switchToDirect?: boolean;
}

interface Props {
  settings: SearchSettings;
  saving: boolean;
  managedUnavailable: boolean;
  updateProvider: (id: string, patch: SearchProviderUpdate) => Promise<boolean>;
  t: Translate;
  /** Wizard mode: the Routing/Websites tabs are hidden, so do not cite them. */
  hideTabChrome?: boolean;
}

/** The secondary line under a connected provider's name. */
function connectedDetail(provider: SearchProviderInfo, t: Translate): string {
  if (provider.route === 'managed') return t('settings.search.routeManaged');
  if (hasBaseUrl(provider)) return provider.base_url || t('settings.search.detailNoUrl');
  if (provider.takes_key)
    return provider.key_configured
      ? t('settings.search.detailOwnKey')
      : provider.key_optional
        ? t('settings.search.detailKeyOptional')
        : t('settings.search.detailNoKey');
  return t('settings.search.routeDirect');
}

/** A catalogue tile's secondary line: what it can do, plus "no key needed" when that applies. */
function tileDetail(provider: SearchProviderInfo, t: Translate): string {
  const roles = rolesSummary(provider, t);
  return provider.key_optional ? `${roles} · ${t('settings.search.detailKeyOptional')}` : roles;
}

const SearchPanelProviders = ({
  settings,
  saving,
  managedUnavailable,
  updateProvider,
  hideTabChrome = false,
  t,
}: Props) => {
  const [dialog, setDialog] = useState<DialogState | null>(null);
  // A provider enabled only over the managed route is not connected when this
  // session has no account behind that route — see `isStrandedManaged`.
  const connected = settings.providers.filter(
    p => p.enabled && !isStrandedManaged(p, managedUnavailable)
  );
  const available = settings.providers.filter(
    p => !p.enabled || isStrandedManaged(p, managedUnavailable)
  );
  const viaTinyHumans = available.filter(p => canUseManaged(p, managedUnavailable));
  const ownKey = available.filter(p => !canUseManaged(p, managedUnavailable));
  const dialogProvider = dialog ? settings.providers.find(p => p.id === dialog.id) : undefined;

  /** Turn a provider on the quickest way it allows. */
  const enable = (provider: SearchProviderInfo) => {
    if (canUseManaged(provider, managedUnavailable)) {
      void updateProvider(provider.id, {
        enabled: true,
        ...(provider.route !== 'managed' ? { route: 'managed' as const } : {}),
      });
      return;
    }
    if (!canUseDirect(provider)) return;
    if (directNeedsSetup(provider)) {
      setDialog({ id: provider.id, mode: 'enable' });
      return;
    }
    void updateProvider(provider.id, {
      enabled: true,
      ...(provider.route !== 'direct' ? { route: 'direct' as const } : {}),
    });
  };

  const rowActions = (provider: SearchProviderInfo): ProviderRowAction[] => {
    const actions: ProviderRowAction[] = [];
    const deepResearchCapable = provider.deep_research_available !== undefined;
    const bothRoutes = provider.routes.includes('managed') && canUseDirect(provider);

    if (bothRoutes && provider.route === 'managed') {
      actions.push({
        label: t('settings.search.actionUseOwnKey'),
        onSelect: () => {
          if (directNeedsSetup(provider))
            setDialog({ id: provider.id, mode: 'key', switchToDirect: true });
          else void updateProvider(provider.id, { route: 'direct' });
        },
      });
    }
    if (bothRoutes && provider.route === 'direct' && canUseManaged(provider, managedUnavailable)) {
      actions.push({
        label: t('settings.search.actionUseManaged'),
        onSelect: () => void updateProvider(provider.id, { route: 'managed' }),
      });
    }
    if (provider.takes_key && (provider.route === 'direct' || deepResearchCapable)) {
      actions.push({
        label: provider.key_configured
          ? t('settings.search.actionReplaceKey')
          : deepResearchCapable && provider.route === 'managed'
            ? t('settings.search.actionAddDeepResearchKey')
            : t('settings.search.actionAddKey'),
        onSelect: () => setDialog({ id: provider.id, mode: 'key' }),
      });
    }
    if (hasBaseUrl(provider)) {
      actions.push({
        label: t('settings.search.actionEditUrl'),
        onSelect: () => setDialog({ id: provider.id, mode: 'url' }),
      });
    }
    if (provider.takes_key && provider.key_configured) {
      actions.push({
        label: t('settings.search.actionRemoveKey'),
        destructive: true,
        onSelect: () => void updateProvider(provider.id, { api_key: '' }),
      });
    }
    return actions;
  };

  const tile = (provider: SearchProviderInfo, detail: string) => {
    // A provider that only runs via TinyHumans cannot be added while signed out.
    const blocked = !canUseManaged(provider, managedUnavailable) && !canUseDirect(provider);
    return (
      <button
        key={provider.id}
        type="button"
        className={tileClass}
        disabled={saving || blocked}
        onClick={() => enable(provider)}
        aria-label={withProvider(t('settings.search.addProviderAria'), provider.label)}
        data-testid={`search-catalog-${provider.id}`}>
        <SearchProviderSwatch id={provider.id} label={provider.label} />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm font-medium text-content">{provider.label}</span>
          <span className="truncate text-[11px] text-content-muted">
            {blocked ? t('settings.search.statusSignInRequired') : detail}
          </span>
        </span>
        <Plus
          className="h-4 w-4 shrink-0 text-content-faint transition-colors group-enabled:group-hover:text-content"
          aria-hidden
        />
      </button>
    );
  };

  return (
    <div className="flex w-full flex-col gap-4">
      <ProviderGroup
        title={t('settings.search.connectedTitle')}
        /* The copy ends "the Routing tab decides which one is tried first".
           The onboarding wizard hides that tab, so it points at something the
           reader cannot see. */
        description={hideTabChrome ? undefined : t('settings.search.connectedDesc')}
        card
        data-testid="search-providers">
        {connected.length === 0 && (
          <li className="px-4 py-3 text-sm text-content-muted" data-testid="search-connected-empty">
            {t('settings.search.connectedEmpty')}
          </li>
        )}
        {connected.map(provider => {
          const testId = `search-provider-${provider.id}`;
          return (
            <ProviderListRow
              key={provider.id}
              slug={provider.id}
              label={provider.label}
              tone=""
              swatch={<SearchProviderSwatch id={provider.id} label={provider.label} />}
              detail={<span data-testid={`${testId}-detail`}>{connectedDetail(provider, t)}</span>}
              detailMono={provider.route === 'direct' && hasBaseUrl(provider)}
              badge={
                <>
                  {provider.status !== 'ready' && (
                    <Badge
                      variant={STATUS_VARIANT[provider.status]}
                      data-testid={`${testId}-status`}>
                      {statusLabel(provider.status, t)}
                    </Badge>
                  )}
                  {provider.deep_research_available && (
                    <Badge variant="primary" data-testid={`${testId}-deep-research`}>
                      {t('settings.search.deepResearchBadge')}
                    </Badge>
                  )}
                </>
              }
              control={
                <Switch
                  id={`${testId}-toggle`}
                  data-testid={`${testId}-toggle`}
                  aria-label={withProvider(t('settings.search.providerToggleAria'), provider.label)}
                  checked
                  disabled={saving}
                  onCheckedChange={next => void updateProvider(provider.id, { enabled: next })}
                />
              }
              actions={rowActions(provider)}
              actionsLabel={withProvider(t('settings.search.rowActions'), provider.label)}
              data-testid={testId}
            />
          );
        })}
      </ProviderGroup>

      {available.length > 0 && (
        <Card
          title={t('settings.search.catalogTitle')}
          description={t('settings.search.catalogDesc')}
          data-testid="search-catalog">
          {viaTinyHumans.length > 0 && (
            <section className="space-y-2.5 p-4" data-testid="search-catalog-managed">
              <div>
                <h4 className="text-sm font-medium text-content">
                  {t('settings.search.catalogManagedTitle')}
                </h4>
                <p className="text-xs text-content-muted">
                  {t('settings.search.catalogManagedHelper')}
                </p>
              </div>
              <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
                {viaTinyHumans.map(p => tile(p, rolesSummary(p, t)))}
              </div>
            </section>
          )}
          {ownKey.length > 0 && (
            <section className="space-y-2.5 p-4" data-testid="search-catalog-direct">
              <div>
                <h4 className="text-sm font-medium text-content">
                  {t('settings.search.catalogOwnTitle')}
                </h4>
                <p className="text-xs text-content-muted">
                  {t('settings.search.catalogOwnHelper')}
                </p>
              </div>
              <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
                {ownKey.map(p => tile(p, tileDetail(p, t)))}
              </div>
            </section>
          )}
        </Card>
      )}

      {dialog && dialogProvider && (
        <SearchPanelConnectDialog
          provider={dialogProvider}
          mode={dialog.mode}
          switchToDirect={dialog.switchToDirect}
          saving={saving}
          onSubmit={patch => updateProvider(dialogProvider.id, patch)}
          onClose={() => setDialog(null)}
          t={t}
        />
      )}
    </div>
  );
};

export default SearchPanelProviders;
