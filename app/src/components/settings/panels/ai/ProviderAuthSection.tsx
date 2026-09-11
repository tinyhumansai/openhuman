/*
 * Provider authentication section.
 *
 * THE SHAPE IS INVERTED FROM WHAT IT WAS. Every provider used to render, all
 * ~15 of them, as pills in one flex-wrap row: the section spent its whole area
 * on the providers nobody chose and gave the one or two actually configured no
 * more prominence than the rest. Now the section answers "what am I using"
 * first — a short list of connected rows, each with room for the state that
 * matters (a masked key, an endpoint) — and demotes "what could I use" into a
 * rich select that only opens when the user asks for it.
 *
 * CLI LOGINS STAY AS FIXED ROWS. Claude Code and Codex do not store a key
 * here; they import a credential another tool already owns, and each brings
 * its own connect control (Claude Code owns a status probe and a modal).
 * Feeding them through the picker would mean driving another component's
 * internal dialog from the outside, so they keep a two-row band of their own
 * whether connected or not.
 */
import { useState } from 'react';
import { LuCircleAlert, LuPlus } from 'react-icons/lu';

import { useT } from '../../../../lib/i18n/I18nContext';
import type { ProviderAuthError } from '../../../../services/api/aiSettingsApi';
import Alert from '../../../ui/Alert';
import Badge from '../../../ui/Badge';
import Button from '../../../ui/Button';
import Card from '../../../ui/Card';
import StatusLine from '../../../ui/StatusLine';
import Switch from '../../../ui/Switch';
import { routingWithProviderRemoved } from '../aiRouting';
import {
  BUILTIN_CLOUD_PROVIDER_SLUGS,
  defaultEndpointForBuiltinCloudProvider,
} from '../builtinCloudProviders';
import { ProviderSetupErrorNotice } from '../ProviderSetupErrorNotice';
import { AddProviderDialog, type ProviderCategory } from './AddProviderDialog';
import {
  type AISettings,
  BUILTIN_PROVIDER_META,
  BUILTIN_RESERVED_SLUGS,
  type CloudProvider,
  formatI18n,
  LOCAL_CHIP_LABEL,
  LOCAL_CHIP_TONE,
  type LocalChipSlug,
  providerToggleAriaLabel,
} from './aiPanelTypes';
import { ClaudeCodeConnect } from './ClaudeCodeStatusCard';
import { ProviderGroup, ProviderListRow, type ProviderRowAction } from './ProviderListRow';

const LOCAL_RUNTIME_SLUGS = ['lmstudio', 'ollama', 'omlx'] as const;

/**
 * CLI logins, in dialog order, each with the slug its credential is STORED
 * under. Codex is the trap: the Codex CLI login is an OpenAI credential, so it
 * lands in `cloudProviders` as `openai` and shows up as the OpenAI row. Keying
 * its "already connected" check on the literal `codex` would never match, and
 * the dialog would keep offering Codex forever. It is also why connecting it
 * busies the `toggle-openai` action.
 */
const CLI_LOGINS = [
  { option: 'claude-code', storedAs: 'claude-code' },
  { option: 'codex', storedAs: 'openai' },
] as const;

/** An endpoint URL reads better in a one-line menu item as just its host. */
const hostOf = (endpoint: string): string => {
  try {
    return new URL(endpoint).host;
  } catch {
    return endpoint;
  }
};

export const ProviderAuthSection = ({
  draft,
  persist,
  loading,
  error,
  busyAction,
  providerAuthErrors,
  providerSaveNotice,
  onDismissProviderSaveNotice,
  onProviderRemoved,
  codexAuthError,
  onConnectCodex,
  onConnectProvider,
  onOpenKeyDialog,
  onAddCustomProvider,
  onEditCustomProvider,
}: {
  draft: AISettings;
  persist: (next: AISettings) => Promise<void>;
  loading: boolean;
  error: string;
  busyAction: string | null;
  providerAuthErrors: ProviderAuthError[];
  providerSaveNotice: { slug: string; message: string } | null;
  /** Unconditional dismiss — the banner's own "Dismiss" button. */
  onDismissProviderSaveNotice: () => void;
  /** Clears the advisory only if it's about the given slug (#5341) — called
   *  when that provider is removed, so an unrelated advisory survives. */
  onProviderRemoved: (slug: string) => void;
  codexAuthError: string | null;
  onConnectCodex: () => void;
  onConnectProvider: (args: {
    slug: string;
    localLabel?: string | null;
    value: string;
    credentialMode: 'api_key' | 'endpoint' | 'endpoint_key' | 'cli_login' | 'oauth';
  }) => Promise<void>;
  onOpenKeyDialog: (slug: string, localLabel: string | null) => void;
  onAddCustomProvider: () => void;
  /** Opens the full editor for a user-defined provider (name, endpoint, key). */
  onEditCustomProvider: (provider: CloudProvider) => void;
}) => {
  const { t } = useT();
  const [addOpen, setAddOpen] = useState(false);

  /** Drop a provider and scrub every routing entry pinned to it, so a workload
   *  cannot keep pointing at a provider that no longer exists. */
  const removeProvider = async (existing: CloudProvider, isLocalRuntime: boolean) => {
    onProviderRemoved(existing.slug);
    const remaining = draft.cloudProviders.filter(cp => cp.id !== existing.id);
    const nextRouting = routingWithProviderRemoved(
      draft.routing,
      { slug: existing.slug, isLocalRuntime },
      remaining
    );
    await persist({ ...draft, cloudProviders: remaining, routing: nextRouting });
  };

  const bySlug = (slug: string) => draft.cloudProviders.find(cp => cp.slug === slug);
  const connectedCloud = BUILTIN_CLOUD_PROVIDER_SLUGS.filter(slug => bySlug(slug));
  const connectedLocal = LOCAL_RUNTIME_SLUGS.filter(slug => bySlug(slug));
  const customProviders = draft.cloudProviders.filter(
    cp => !BUILTIN_RESERVED_SLUGS.includes(cp.slug)
  );

  // Three categories, each a different question rather than a slice of one:
  // cloud wants an API key, local wants an endpoint on this machine, CLI wants
  // nothing because another tool already holds the credential. Custom is not
  // here — it is a single action, handled by the dialog's own button.
  const providerCategories: ProviderCategory[] = [
    {
      id: 'cloud',
      title: t('settings.ai.providers.groupCloud'),
      placeholder: t('settings.ai.providers.placeholderCloud'),
      helper: t('settings.ai.providers.helperCloud'),
      options: BUILTIN_CLOUD_PROVIDER_SLUGS.filter(slug => !bySlug(slug)).map(slug => ({
        slug,
        label: BUILTIN_PROVIDER_META[slug]?.label ?? slug,
        tone: BUILTIN_PROVIDER_META[slug]?.tone ?? '',
        detail: hostOf(defaultEndpointForBuiltinCloudProvider(slug)),
      })),
    },
    {
      id: 'local',
      title: t('settings.ai.providers.groupLocal'),
      placeholder: t('settings.ai.providers.placeholderLocal'),
      helper: t('settings.ai.providers.helperLocal'),
      options: LOCAL_RUNTIME_SLUGS.filter(slug => !bySlug(slug)).map(slug => ({
        slug,
        label: LOCAL_CHIP_LABEL[slug as LocalChipSlug],
        tone: LOCAL_CHIP_TONE[slug as LocalChipSlug],
        detail: t('settings.ai.providers.localDetail'),
      })),
    },
    {
      id: 'cli',
      title: t('settings.ai.providers.groupCli'),
      placeholder: t('settings.ai.providers.placeholderCli'),
      helper: t('settings.ai.providers.helperCli'),
      options: CLI_LOGINS.filter(cli => !bySlug(cli.storedAs)).map(cli => ({
        slug: cli.option,
        label:
          cli.option === 'claude-code'
            ? t('settings.ai.claudeCode.button')
            : t('settings.ai.codexAuthButton'),
        tone: BUILTIN_PROVIDER_META[cli.storedAs]?.tone ?? '',
        detail: t('settings.ai.providers.cliDetail'),
      })),
    },
  ];

  const handlePick = (slug: string) => {
    setAddOpen(false);
    if (slug === 'codex') {
      onConnectCodex();
      return;
    }
    if (slug === 'claude-code') {
      void onConnectProvider({ slug, value: 'cli_login', credentialMode: 'cli_login' });
      return;
    }
    const localLabel = LOCAL_RUNTIME_SLUGS.includes(slug as (typeof LOCAL_RUNTIME_SLUGS)[number])
      ? LOCAL_CHIP_LABEL[slug as LocalChipSlug]
      : null;
    onOpenKeyDialog(slug, localLabel);
  };

  const claudeCodeConnected = Boolean(bySlug('claude-code'));

  return (
    <>
      <div className="flex w-full flex-col gap-4 py-4">
        <div className="flex w-full flex-col gap-4 px-4">
          {/* ─── Rejected-key notices ───────────────────────────────────────
            A BYO key the provider rejected at runtime (401/403). Surfaced
            here, next to the key editor, because the failing path is often a
            silent background loop and the raw error is demoted from Sentry. */}
          {providerAuthErrors.length > 0 && (
            <div className="flex w-full flex-col gap-2">
              {providerAuthErrors.map(err => (
                <ProviderSetupErrorNotice key={err.provider} error={err.message} />
              ))}
            </div>
          )}

          {/* #5339: non-fatal "key saved, but provider unreachable" advisory.
            Amber (not coral): the save succeeded, only reachability is in
            question. */}
          {providerSaveNotice && (
            <Alert variant="warning" role="status" className="items-start gap-2 px-3 py-2 text-xs">
              <LuCircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span className="flex-1">{providerSaveNotice.message}</span>
              <Button
                type="button"
                variant="tertiary"
                size="xs"
                className="shrink-0 font-medium normal-case underline-offset-2 hover:underline"
                onClick={onDismissProviderSaveNotice}>
                {t('common.dismiss')}
              </Button>
            </Alert>
          )}

          {loading && <div className="text-xs text-content-muted">{t('common.loading')}</div>}
          {error && <StatusLine saving={false} error={error} savedNote={null} savingLabel="" />}
        </div>

        <Card title={t('settings.ai.llmProviders')} description={t('settings.ai.llmProvidersDesc')}>
          <div className="flex justify-end px-4 py-2">
            <Button
              type="button"
              variant="primary"
              size="xs"
              leadingIcon={<LuPlus className="h-3.5 w-3.5" />}
              onClick={() => setAddOpen(true)}
              data-testid="add-provider-open">
              {t('settings.ai.providers.addProvider')}
            </Button>
          </div>
        </Card>

        {/* ─── Connected ────────────────────────────────────────────────────
          Managed leads and is always present. #3760: it renders a badge, not a
          disabled toggle — a locked switch reads as switchable-but-broken and
          invites a fight the user cannot win. */}
        <ProviderGroup
          title={t('settings.ai.providers.groupConnected')}
          card
          data-testid="provider-group-connected">
          <ProviderListRow
            slug="openhuman"
            label={t('settings.ai.routing.managed')}
            tone={BUILTIN_PROVIDER_META.openhuman?.tone ?? ''}
            detail={t('settings.ai.providers.managedDetail')}
            control={<Badge variant="success">{t('settings.ai.routing.managedAlwaysOn')}</Badge>}
            data-testid="provider-row-openhuman"
          />

          {connectedCloud.map(slug => {
            const existing = bySlug(slug)!;
            const meta = BUILTIN_PROVIDER_META[slug];
            const label = meta?.label ?? slug;
            const actions: ProviderRowAction[] = [
              {
                label: t('settings.ai.providers.replaceKey'),
                onSelect: () => onOpenKeyDialog(slug, null),
              },
            ];
            return (
              <ProviderListRow
                key={slug}
                slug={slug}
                label={label}
                tone={meta?.tone ?? ''}
                detail={existing.maskedKey || hostOf(existing.endpoint)}
                detailMono
                control={
                  <Switch
                    id={`provider-toggle-${slug}`}
                    checked
                    onCheckedChange={async () => await removeProvider(existing, false)}
                    disabled={busyAction === `toggle-${slug}`}
                    aria-label={providerToggleAriaLabel(t, true, label)}
                  />
                }
                actions={actions}
                actionsLabel={formatI18n(t('settings.ai.providers.rowActions'), {
                  provider: label,
                })}
                data-testid={`provider-row-${slug}`}
              />
            );
          })}

          {customProviders.map(existing => (
            <ProviderListRow
              key={existing.id}
              slug={existing.slug}
              label={existing.label}
              tone={BUILTIN_PROVIDER_META.custom?.tone ?? ''}
              detail={hostOf(existing.endpoint) || existing.maskedKey}
              detailMono
              badge={<Badge variant="primary">{t('settings.ai.providers.custom')}</Badge>}
              control={
                <Switch
                  id={`provider-toggle-${existing.slug}`}
                  checked
                  onCheckedChange={async () => await removeProvider(existing, false)}
                  disabled={busyAction === `toggle-${existing.slug}`}
                  aria-label={providerToggleAriaLabel(t, true, existing.label)}
                />
              }
              actions={[
                { label: t('common.edit'), onSelect: () => onEditCustomProvider(existing) },
                {
                  label: t('common.remove'),
                  destructive: true,
                  onSelect: () => void removeProvider(existing, false),
                },
              ]}
              actionsLabel={formatI18n(t('settings.ai.providers.rowActions'), {
                provider: existing.label,
              })}
              data-testid={`provider-row-${existing.slug}`}
            />
          ))}

          {connectedLocal.map(slug => {
            const existing = bySlug(slug)!;
            const label = LOCAL_CHIP_LABEL[slug as LocalChipSlug];
            return (
              <ProviderListRow
                key={slug}
                slug={slug}
                label={label}
                tone={LOCAL_CHIP_TONE[slug as LocalChipSlug]}
                // The endpoint is the thing that breaks on a local runtime, so
                // it is shown in full rather than reduced to a host.
                detail={existing.endpoint || t('settings.ai.providers.connected')}
                detailMono
                control={
                  <Switch
                    id={`local-runtime-toggle-${slug}`}
                    checked
                    onCheckedChange={async () => await removeProvider(existing, true)}
                    disabled={busyAction === `toggle-${slug}`}
                    aria-label={providerToggleAriaLabel(t, true, label)}
                  />
                }
                actions={[
                  {
                    label: t('settings.ai.editEndpoint'),
                    onSelect: () => onOpenKeyDialog(slug, label),
                  },
                ]}
                actionsLabel={formatI18n(t('settings.ai.providers.rowActions'), {
                  provider: label,
                })}
                data-testid={`provider-row-${slug}`}
              />
            );
          })}
        </ProviderGroup>

        {/* ─── CLI logins ────────────────────────────────────────────────
          Only Claude Code earns a row here, and only once connected: it owns a
          status probe and a modal that no other provider has, and disconnecting
          goes through them. Codex deliberately has NO row — its credential is
          stored as `openai`, so it is already the OpenAI row above, and a second
          row would imply a second connection the user could remove separately. */}
        {claudeCodeConnected && (
          <ProviderGroup
            title={t('settings.ai.providers.groupCli')}
            card
            data-testid="provider-group-cli">
            <ProviderListRow
              slug="claude-code"
              label={t('settings.ai.claudeCode.button')}
              tone={
                BUILTIN_PROVIDER_META['claude-code']?.tone ??
                BUILTIN_PROVIDER_META.custom?.tone ??
                ''
              }
              detail={t('settings.ai.providers.cliDetail')}
              control={
                <ClaudeCodeConnect
                  connected
                  busy={busyAction === 'toggle-claude-code'}
                  onConnect={() =>
                    onConnectProvider({
                      slug: 'claude-code',
                      value: 'cli_login',
                      credentialMode: 'cli_login',
                    })
                  }
                  onDisconnect={async () => {
                    const existing = bySlug('claude-code');
                    if (existing) await removeProvider(existing, false);
                  }}
                />
              }
              data-testid="provider-row-claude-code"
            />
          </ProviderGroup>
        )}

        <div className="flex flex-col gap-3 px-4">
          {codexAuthError ? <ProviderSetupErrorNotice error={codexAuthError} /> : null}

          {/* #3760: point users who want a local model at the Routing card
            below, rather than letting them hunt for a Managed off switch. */}
          <p className="text-xs text-content-muted">{t('settings.ai.routing.managedHint')}</p>
        </div>
      </div>

      {addOpen && (
        <AddProviderDialog
          categories={providerCategories}
          onPick={handlePick}
          onAddCustom={() => {
            setAddOpen(false);
            onAddCustomProvider();
          }}
          onClose={() => setAddOpen(false)}
        />
      )}
    </>
  );
};

export default ProviderAuthSection;
