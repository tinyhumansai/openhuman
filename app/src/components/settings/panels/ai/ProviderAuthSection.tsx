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
import { Plus } from 'lucide-react';
import { useState } from 'react';

import { useManagedInferenceAvailable } from '../../../../hooks/useLocalSession';
import { useT } from '../../../../lib/i18n/I18nContext';
import type { ProviderAuthError } from '../../../../services/api/aiSettingsApi';
import type { OciSettings } from '../../../../utils/tauriCommands/config';
import Alert, { AlertDescription } from '../../../ui/Alert';
import Badge from '../../../ui/Badge';
import Button from '../../../ui/Button';
import { CenteredLoadingState } from '../../../ui/LoadingState';
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
import { OCI_PROVIDER_SLUG, ociSettingsSummary } from './ociSettings';
import { OciSettingsDialog } from './OciSettingsDialog';
import { ProviderCatalog } from './ProviderCatalog';
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

/** The one "Add provider" control, shared by the section and any host header. */
export const AddProviderButton = ({ onClick }: { onClick: () => void }) => {
  const { t } = useT();
  return (
    <Button
      type="button"
      variant="primary"
      size="sm"
      leadingIcon={<Plus className="h-3.5 w-3.5" aria-hidden />}
      onClick={onClick}
      data-testid="add-provider-open">
      {t('settings.ai.providers.addProvider')}
    </Button>
  );
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
  addOpen: controlledAddOpen,
  onAddOpenChange,
  hideAddButton = false,
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
    credentialMode: 'api_key' | 'endpoint' | 'endpoint_key' | 'cli_login' | 'oauth' | 'oci';
    oci?: OciSettings;
  }) => Promise<void>;
  onOpenKeyDialog: (slug: string, localLabel: string | null) => void;
  onAddCustomProvider: () => void;
  /** Opens the full editor for a user-defined provider (name, endpoint, key). */
  onEditCustomProvider: (provider: CloudProvider) => void;
  /**
   * Host-controlled "Add provider" dialog. When a host owns the page header it
   * places the button there and drives the dialog through these; the section
   * then renders no button of its own. Left undefined, the section keeps a
   * local button and state (the embedded onboarding case).
   */
  addOpen?: boolean;
  onAddOpenChange?: (open: boolean) => void;
  /**
   * Hide the standalone "Add a provider" button.
   *
   * The onboarding wizard renders the provider catalog inline, so the button
   * opened a dialog onto a list already on screen — two ways to do one thing,
   * and the loudest control on a step whose primary action is Continue.
   */
  hideAddButton?: boolean;
}) => {
  const { t } = useT();
  const managedAvailable = useManagedInferenceAvailable();
  const [localAddOpen, setLocalAddOpen] = useState(false);
  // Slug of the OCI entry whose settings dialog is open: the built-in `oci`
  // or a custom slug with `authStyle: 'oci'` (e.g. `oci-cohere`).
  const [ociDialogSlug, setOciDialogSlug] = useState<string | null>(null);
  const hostControlsAdd = controlledAddOpen !== undefined;
  const addOpen = controlledAddOpen ?? localAddOpen;
  const setAddOpen = (open: boolean) => {
    if (!hostControlsAdd) setLocalAddOpen(open);
    onAddOpenChange?.(open);
  };

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
  // An empty "Connected" card is a heading over nothing. It used to be
  // guaranteed non-empty by the always-on managed row; now that the row is
  // gated on a real session, a local user with no keys yet got a bare box.
  const hasConnected =
    managedAvailable ||
    connectedCloud.length > 0 ||
    customProviders.length > 0 ||
    connectedLocal.length > 0;

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
    if (slug === OCI_PROVIDER_SLUG) {
      // OCI is configured through its own settings form (auth mode, profile,
      // region, compartment), not the single-field key dialog.
      setOciDialogSlug(OCI_PROVIDER_SLUG);
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
      <div className="flex w-full flex-col gap-4">
        {/* ─── Notices ────────────────────────────────────────────────────
          A BYO key the provider rejected at runtime (401/403) is surfaced
          here, next to the key editor, because the failing path is often a
          silent background loop and the raw error is demoted from Sentry. */}
        {providerAuthErrors.map(err => (
          <ProviderSetupErrorNotice key={err.provider} error={err.message} />
        ))}

        {/* #5339: non-fatal "key saved, but provider unreachable" advisory.
          Amber (not coral): the save succeeded, only reachability is in
          question. */}
        {providerSaveNotice && (
          <Alert variant="warning" density="compact" role="status" className="items-center">
            <AlertDescription className="flex-1">{providerSaveNotice.message}</AlertDescription>
            <Button
              type="button"
              variant="tertiary"
              size="xs"
              className="shrink-0"
              onClick={onDismissProviderSaveNotice}>
              {t('common.dismiss')}
            </Button>
          </Alert>
        )}

        {error && (
          <Alert variant="destructive" density="compact">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}
        {codexAuthError ? <ProviderSetupErrorNotice error={codexAuthError} /> : null}

        {!hostControlsAdd && !hideAddButton && (
          <div className="flex justify-end">
            <AddProviderButton onClick={() => setAddOpen(true)} />
          </div>
        )}

        {loading && <CenteredLoadingState label={t('common.loading')} />}

        {/* ─── Connected ────────────────────────────────────────────────────
          Managed leads WHEN IT EXISTS. It used to render unconditionally, so a
          self-hosting user — signed in with "Continue Locally", no TinyHumans
          account behind the session — was told OpenHuman "chooses a model for
          each task" and shown an "Always on" badge for a service they do not
          have. That is the first thing the inference step shows them, and it
          is false. The row now follows the same rule the search and embeddings
          panels already apply.

          #3760: it renders a badge, not a disabled toggle — a locked switch
          reads as switchable-but-broken and invites a fight the user cannot
          win. */}
        {hasConnected && (
          <ProviderGroup
            title={t('settings.ai.providers.groupConnected')}
            /* The description says "Managed is always on as a fallback. Choose
             which provider each task uses on the Routing tab." Both halves are
             wrong without a managed session: there is no managed fallback, and
             the onboarding wizard hides the Routing tab. Shown only where both
             are true. */
            description={managedAvailable ? t('settings.ai.providers.connectedDesc') : undefined}
            card
            data-testid="provider-group-connected">
            {managedAvailable && (
              <ProviderListRow
                slug="openhuman"
                label={t('settings.ai.routing.managed')}
                tone={BUILTIN_PROVIDER_META.openhuman?.tone ?? ''}
                detail={t('settings.ai.providers.managedDetail')}
                control={
                  <Badge variant="success">{t('settings.ai.routing.managedAlwaysOn')}</Badge>
                }
                data-testid="provider-row-openhuman"
              />
            )}

            {connectedCloud.map(slug => {
              const existing = bySlug(slug)!;
              const meta = BUILTIN_PROVIDER_META[slug];
              const label = meta?.label ?? slug;
              const isOci = slug === OCI_PROVIDER_SLUG;
              const actions: ProviderRowAction[] = [
                isOci
                  ? {
                      label: t('settings.ai.oci.editSettings'),
                      onSelect: () => setOciDialogSlug(slug),
                    }
                  : {
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
                  detail={
                    (isOci ? ociSettingsSummary(existing.oci, t) : '') ||
                    existing.maskedKey ||
                    hostOf(existing.endpoint)
                  }
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
                detail={
                  (existing.authStyle === 'oci' ? ociSettingsSummary(existing.oci, t) : '') ||
                  hostOf(existing.endpoint) ||
                  existing.maskedKey
                }
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
                  {
                    label: t('common.edit'),
                    // OCI entries keep their settings in the `oci` block, which
                    // only the OCI dialog edits; the generic editor would drop it.
                    onSelect: () =>
                      existing.authStyle === 'oci'
                        ? setOciDialogSlug(existing.slug)
                        : onEditCustomProvider(existing),
                  },
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
        )}

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

        {/* ─── Catalogue: everything not yet connected, as tiles ──────── */}
        <ProviderCatalog
          categories={providerCategories}
          onPick={handlePick}
          onAddCustom={onAddCustomProvider}
        />
      </div>

      {ociDialogSlug && (
        <OciSettingsDialog
          label={
            BUILTIN_PROVIDER_META[ociDialogSlug]?.label ??
            bySlug(ociDialogSlug)?.label ??
            ociDialogSlug
          }
          initial={bySlug(ociDialogSlug)?.oci}
          hasStoredKey={Boolean(bySlug(ociDialogSlug)?.maskedKey.startsWith('••••'))}
          onCancel={() => setOciDialogSlug(null)}
          onSubmit={async (oci, apiKey) => {
            await onConnectProvider({
              slug: ociDialogSlug,
              value: apiKey,
              credentialMode: 'oci',
              oci,
            });
            setOciDialogSlug(null);
          }}
        />
      )}

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
