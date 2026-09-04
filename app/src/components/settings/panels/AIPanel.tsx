/*
 * AI settings — three orthogonal sections:
 *   1. Cloud providers (credentials + primary selection)
 *   2. Local provider (Ollama runtime + installed models)
 *   3. Workload routing (8-row matrix; per-workload provider + model)
 *
 * "Primary cloud" is an abstraction: any workload set to "Primary" inherits
 * whichever cloud provider is currently marked primary. Overrides are explicit
 * per row, so the resolved provider+model is always rendered inline.
 *
 * This file is a thin composition — every section lives in `./ai/*`.
 */
import { useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  clearCloudProviderKey,
  upsertModelRegistryVision,
} from '../../../services/api/aiSettingsApi';
import { connectOpenRouterViaOAuth } from '../../../utils/openrouterOAuth';
import PanelPage from '../../layout/PanelPage';
import Alert from '../../ui/Alert';
import Button from '../../ui/Button';
import Card from '../../ui/Card';
import { ModalShell } from '../../ui/ModalShell';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import {
  buildRoutingDiffSummary,
  BUILTIN_PROVIDER_META,
  type CloudProvider,
  defaultEndpointFor,
  formatI18n,
  inferRoutingMode,
  inferSharedModelRef,
  ROUTING_WORKLOAD_IDS,
  routingWithAllWorkloads,
  type WorkloadId,
  WORKLOADS,
} from './ai/aiPanelTypes';
import { BackgroundLoopControls } from './ai/BackgroundLoopControls';
import { CloudProviderEditor } from './ai/CloudProviderEditor';
import { CustomRoutingDialog } from './ai/CustomRoutingDialog';
import { GlobalOwnModelSelector } from './ai/GlobalOwnModelSelector';
import { ProviderAuthSection } from './ai/ProviderAuthSection';
import { ProviderKeyDialog } from './ai/ProviderConnectControls';
import { RoutingModeCards } from './ai/RoutingModeCards';
import { SaveBar } from './ai/SaveBar';
import { useAISettings, useInstalledModels, useOllamaStatus } from './ai/useAISettingsState';
import { useCloudProviderEditorSubmit } from './ai/useCloudProviderEditorSubmit';
import { useProviderConnect } from './ai/useProviderConnect';
import { WorkloadRow } from './ai/WorkloadRow';
import { WorkloadTable } from './ai/WorkloadTable';
import { useReembedBackfillModal } from './useReembedBackfillModal';

export type { CloudProvider, ProviderRef, RoutingMap } from './ai/aiPanelTypes';
export { buildRoutingDiffSummary, BackgroundLoopControls };

export type AIPanelTab = 'providers' | 'routing';

interface AIPanelProps {
  /** When true, the panel is rendered embedded inside another flow (e.g. the
   *  onboarding custom wizard) and skips its own SettingsHeader chrome so the
   *  host frame's title/back controls aren't duplicated. */
  embedded?: boolean;
  /** Selected section when the host owns the page-level chip tabs. */
  tab?: AIPanelTab;
  /** Called when the selected section changes. */
  onTabChange?: (tab: AIPanelTab) => void;
  /** Suppress PanelPage's internal tab chrome for a host-rendered chip row. */
  hideTabChrome?: boolean;
}

const AIPanel = ({
  embedded = false,
  tab: controlledTab,
  onTabChange,
  hideTabChrome = false,
}: AIPanelProps = {}) => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const { saved, draft, isDirty, save, persist, discard, loading, error, reload } = useAISettings();
  // #1574 §4b: advisory re-embed modal, driven by the backend status RPC.
  const { reembed, handleSave, dismissReembed } = useReembedBackfillModal(save);
  const ollama = useOllamaStatus();
  const installed = useInstalledModels(ollama.snapshot);
  const [editing, setEditing] = useState<CloudProvider | 'new' | null>(null);
  // Which workload's "Custom" dialog is currently open (null = closed).
  const [pickerFor, setPickerFor] = useState<WorkloadId | null>(null);
  const [routingEditorMode, setRoutingEditorMode] = useState<'own' | 'custom' | null>(null);
  // Which provider slug's API-key dialog is currently open (null = closed).
  const [keyDialogFor, setKeyDialogFor] = useState<string | null>(null);
  // When the user toggles LM Studio / Ollama (local runtimes), we need to
  // remember which label to attach to the upserted provider. Cleared when
  // the dialog closes.
  const [pendingLocalLabel, setPendingLocalLabel] = useState<string | null>(null);
  const openRouterOauthAbortRef = useRef<AbortController | null>(null);
  // Two orthogonal jobs on one page: WHICH providers exist, and WHICH one
  // each workload uses. They were stacked, so the routing controls sat below
  // a provider list whose length varies with the user's setup. Tabs give each
  // the full pane and a stable position.
  const [uncontrolledTab, setUncontrolledTab] = useState<AIPanelTab>('providers');
  const tab = controlledTab ?? uncontrolledTab;
  const handleTabChange = (nextTab: AIPanelTab) => {
    if (controlledTab === undefined) setUncontrolledTab(nextTab);
    onTabChange?.(nextTab);
  };

  const {
    busyAction,
    setBusyAction,
    codexAuthError,
    providerAuthErrors,
    providerSaveNotice,
    setProviderSaveNotice,
    connectProvider,
    connectOpenAiViaCodexAuth,
  } = useProviderConnect({
    draft,
    saved,
    persist,
    t,
    onConnected: () => {
      setKeyDialogFor(null);
      setPendingLocalLabel(null);
    },
  });

  const submitCloudProviderEdit = useCloudProviderEditorSubmit({
    editing,
    draft,
    saved,
    persist,
    t,
    onDone: () => setEditing(null),
  });

  const diffSummary = buildRoutingDiffSummary(saved.routing, draft.routing, t);
  const chatRows = WORKLOADS.filter(w => w.group === 'chat');
  const bgRows = WORKLOADS.filter(w => w.group === 'background');
  const inferredRoutingModeRaw = inferRoutingMode(draft.routing);
  // Routing mode is derived purely from the workload routing map, not from the
  // set of configured providers: saving a provider key only adds a
  // `cloudProviders` entry, it does not rewrite `routing`. Surfaced for
  // support diagnostics (the recurring "my key is added but not used" question).
  const configuredWithKey = draft.cloudProviders.filter(p => p.maskedKey.startsWith('••••'));
  console.debug('[ai-settings][routing] inferred mode', {
    mode: inferredRoutingModeRaw,
    routing: ROUTING_WORKLOAD_IDS.map(id => `${id}:${draft.routing[id]?.kind}`),
    configured_providers: draft.cloudProviders.map(p => p.slug),
    configured_with_key: configuredWithKey.map(p => p.slug),
    configured_but_managed: inferredRoutingModeRaw === 'managed' && configuredWithKey.length > 0,
  });
  const effectiveRoutingMode =
    routingEditorMode === 'own'
      ? 'own'
      : routingEditorMode === 'custom'
        ? 'custom'
        : inferredRoutingModeRaw;
  const sharedModelRef = inferSharedModelRef(draft.routing);

  return (
    <>
      <PanelPage
        className="z-10"
        contentClassName=""
        description={embedded ? undefined : t('pages.settings.ai.llmDesc')}
        leading={embedded ? undefined : <SettingsBackButton onBack={navigateBack} />}
        tabsAriaLabel={t('pages.settings.ai.llm')}
        tabsTestIdPrefix="ai-tab"
        value={tab}
        onChange={handleTabChange}
        hideTabChrome={hideTabChrome}
        scrollable={!hideTabChrome}
        tabs={[
          {
            id: 'providers',
            label: t('settings.ai.llmProviders'),
            contentClassName: embedded || hideTabChrome ? '' : 'p-4',
            content: (
              <div className="flex w-full flex-col">
                <ProviderAuthSection
                  draft={draft}
                  persist={persist}
                  loading={loading}
                  error={error}
                  busyAction={busyAction}
                  providerAuthErrors={providerAuthErrors}
                  providerSaveNotice={providerSaveNotice}
                  onDismissProviderSaveNotice={() => setProviderSaveNotice(null)}
                  onProviderRemoved={slug =>
                    setProviderSaveNotice(prev => (prev?.slug === slug ? null : prev))
                  }
                  codexAuthError={codexAuthError}
                  onConnectCodex={() => void connectOpenAiViaCodexAuth()}
                  onConnectProvider={connectProvider}
                  onOpenKeyDialog={(slug, localLabel) => {
                    setKeyDialogFor(slug);
                    setPendingLocalLabel(localLabel);
                  }}
                  onAddCustomProvider={() => setEditing('new')}
                  onEditCustomProvider={provider => setEditing(provider)}
                />
                {isDirty && (
                  <SaveBar
                    diffSummary={diffSummary}
                    changeCount={diffSummary.length}
                    onSave={() => void handleSave()}
                    onDiscard={discard}
                  />
                )}
              </div>
            ),
          },
          {
            id: 'routing',
            label: t('settings.ai.routing'),
            contentClassName: embedded || hideTabChrome ? '' : 'p-4',
            content: (
              <div className="flex w-full flex-col gap-4">
                {/* ═══════════════════════════════════════════════════════════════
              ROUTING — top-level routing mode. Managed = OpenHuman decides.
              Own = one provider/model for everything. Custom = fine-grained
              per-workload routing.
              ═══════════════════════════════════════════════════════════════ */}
                <>
                  <RoutingModeCards
                    effectiveRoutingMode={effectiveRoutingMode}
                    onSelectManaged={() => {
                      setRoutingEditorMode(null);
                      void persist({
                        ...draft,
                        routing: routingWithAllWorkloads({ kind: 'openhuman' }),
                      });
                    }}
                    onSelectOwn={() => setRoutingEditorMode('own')}
                    onSelectCustom={() => setRoutingEditorMode('custom')}
                  />

                  {effectiveRoutingMode === 'managed' ? (
                    <Card className="w-full">
                      <Alert variant="success">{t('settings.ai.routing.managedMsg')}</Alert>
                    </Card>
                  ) : null}

                  {effectiveRoutingMode === 'own' ? (
                    <GlobalOwnModelSelector
                      current={sharedModelRef}
                      saved={inferSharedModelRef(saved.routing)}
                      cloudProviders={draft.cloudProviders}
                      localModels={installed}
                      ollamaRunning={ollama.state === 'running' || ollama.state === 'degraded'}
                      modelRegistry={draft.modelRegistry}
                      onApply={async (next, vision) => {
                        const reg =
                          next.kind === 'cloud'
                            ? { slug: next.providerSlug, model: next.model }
                            : next.kind === 'local'
                              ? { slug: 'ollama', model: next.model }
                              : next.kind === 'claude-code'
                                ? { slug: 'claude-code', model: next.model }
                                : null;
                        await persist({
                          ...draft,
                          routing: routingWithAllWorkloads(next),
                          modelRegistry: reg
                            ? upsertModelRegistryVision(
                                draft.modelRegistry,
                                reg.slug,
                                reg.model,
                                vision
                              )
                            : draft.modelRegistry,
                        });
                      }}
                    />
                  ) : null}

                  {effectiveRoutingMode === 'custom' ? (
                    <>
                      <Card className="w-full">
                        <WorkloadTable
                          title={t('settings.ai.routing.chatAndConversations')}
                          description={t('settings.ai.routing.chatDesc')}>
                          {chatRows.map(w => (
                            <WorkloadRow
                              key={w.id}
                              workload={w}
                              ref_={draft.routing[w.id]}
                              cloudProviders={draft.cloudProviders}
                              onCustomClick={() => setPickerFor(w.id)}
                            />
                          ))}
                        </WorkloadTable>
                      </Card>

                      <Card className="w-full">
                        <WorkloadTable
                          title={t('settings.ai.routing.backgroundTasks')}
                          description={t('settings.ai.routing.bgTasksDesc')}>
                          {bgRows.map(w => (
                            <WorkloadRow
                              key={w.id}
                              workload={w}
                              ref_={draft.routing[w.id]}
                              cloudProviders={draft.cloudProviders}
                              onCustomClick={() => setPickerFor(w.id)}
                            />
                          ))}
                        </WorkloadTable>
                      </Card>
                    </>
                  ) : null}
                </>
                {isDirty && (
                  <SaveBar
                    diffSummary={diffSummary}
                    changeCount={diffSummary.length}
                    onSave={() => void handleSave()}
                    onDiscard={discard}
                  />
                )}
              </div>
            ),
          },
        ]}
      />
      {/* Informational, not a decision: one acknowledging action and no
        second choice. That rules out `AlertDialog`, whose own contract
        requires rendering a Cancel — offering "Cancel" for a notice the user
        can only acknowledge invents a branch that does not exist. `Dialog`
        via `ModalShell` is the right primitive, and it still brings the focus
        trap, scroll lock and Escape handling. */}
      {reembed.open && (
        <ModalShell
          title={t('settings.ai.reindexingMemory')}
          titleId="ai-reembed-dialog-title"
          onClose={dismissReembed}
          maxWidthClassName="max-w-sm"
          footer={
            <div className="flex justify-end">
              <Button variant="primary" size="sm" onClick={dismissReembed}>
                {t('common.ok')}
              </Button>
            </div>
          }>
          <div className="text-sm text-content-secondary">
            {formatI18n(t('settings.ai.reindexingMemoryMessage'), { pending: reembed.pending })}
          </div>
        </ModalShell>
      )}

      {editing && (
        <CloudProviderEditor
          initial={editing === 'new' ? null : editing}
          existingSlugs={draft.cloudProviders
            .filter(p => p.id !== (editing === 'new' ? '' : editing.id))
            .map(p => p.slug)}
          onClose={() => setEditing(null)}
          onSubmit={async (next, apiKey, opts) => {
            setBusyAction('save-provider');
            try {
              await submitCloudProviderEdit(next, apiKey, opts);
            } finally {
              setBusyAction(null);
            }
          }}
          onClearKey={async slug => {
            // Clearing this provider's key drops its own advisory (#5341).
            setProviderSaveNotice(prev => (prev?.slug === slug ? null : prev));
            try {
              await clearCloudProviderKey(slug);
              await reload();
            } catch (err) {
              const msg = err instanceof Error ? err.message : String(err);
              console.warn('[ai-settings] clearCloudProviderKey failed', msg);
            }
          }}
        />
      )}

      {pickerFor &&
        (() => {
          const current = draft.routing[pickerFor];
          const workload = WORKLOADS.find(candidate => candidate.id === pickerFor);
          if (!workload) return null;
          return (
            <CustomRoutingDialog
              workload={workload}
              initial={current}
              cloudProviders={draft.cloudProviders}
              localModels={installed}
              ollamaRunning={ollama.state === 'running' || ollama.state === 'degraded'}
              modelRegistry={draft.modelRegistry}
              onClose={() => setPickerFor(null)}
              onSubmit={(next, vision) => {
                const registryTarget =
                  next.kind === 'cloud'
                    ? { slug: next.providerSlug, model: next.model }
                    : next.kind === 'local'
                      ? { slug: 'ollama', model: next.model }
                      : next.kind === 'claude-code'
                        ? { slug: 'claude-code', model: next.model }
                        : null;
                void persist({
                  ...draft,
                  routing: { ...draft.routing, [pickerFor]: next },
                  modelRegistry: registryTarget
                    ? upsertModelRegistryVision(
                        draft.modelRegistry,
                        registryTarget.slug,
                        registryTarget.model,
                        vision
                      )
                    : draft.modelRegistry,
                });
                setPickerFor(null);
              }}
            />
          );
        })()}

      {keyDialogFor && (
        <ProviderKeyDialog
          slug={keyDialogFor}
          label={pendingLocalLabel ?? BUILTIN_PROVIDER_META[keyDialogFor]?.label ?? keyDialogFor}
          isLocalRuntime={Boolean(pendingLocalLabel)}
          // OMLX is the only endpoint+key local runtime: render both an endpoint
          // field (prefilled with the localhost default) and an API key field.
          endpointKeyMode={keyDialogFor === 'omlx'}
          initialValue={
            pendingLocalLabel
              ? (draft.cloudProviders.find(cp => cp.slug === keyDialogFor)?.endpoint ??
                (keyDialogFor === 'omlx' ? defaultEndpointFor('omlx') : undefined))
              : undefined
          }
          oauthAction={
            keyDialogFor === 'openrouter' && !pendingLocalLabel
              ? {
                  label: t('settings.ai.signInWithOpenRouter'),
                  onClick: async () => {
                    const controller = new AbortController();
                    openRouterOauthAbortRef.current = controller;
                    try {
                      const apiKey = await connectOpenRouterViaOAuth({ signal: controller.signal });
                      await connectProvider({
                        slug: 'openrouter',
                        value: apiKey,
                        credentialMode: 'oauth',
                      });
                    } finally {
                      if (openRouterOauthAbortRef.current === controller) {
                        openRouterOauthAbortRef.current = null;
                      }
                    }
                  },
                }
              : null
          }
          onCancel={() => {
            openRouterOauthAbortRef.current?.abort();
            openRouterOauthAbortRef.current = null;
            setKeyDialogFor(null);
            setPendingLocalLabel(null);
          }}
          onSubmit={async (value, endpoint) =>
            await connectProvider({
              slug: keyDialogFor,
              localLabel: pendingLocalLabel,
              // In endpoint_key (OMLX) mode the dialog hands back the API key as
              // `value` and the endpoint URL as `endpoint`.
              value,
              endpoint,
              credentialMode:
                keyDialogFor === 'omlx'
                  ? 'endpoint_key'
                  : pendingLocalLabel
                    ? 'endpoint'
                    : 'api_key',
            })
          }
        />
      )}
    </>
  );
};

export default AIPanel;
