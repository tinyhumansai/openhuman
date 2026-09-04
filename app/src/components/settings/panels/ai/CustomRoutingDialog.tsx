/*
 * Custom-routing dialog — opened when the user clicks "Custom" on a workload.
 * Lets them pick a provider (cloud or local) and the specific model id.
 */
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import {
  describeProviderVerificationFailure,
  type ModelRegistryEntry,
  modelRegistryVision,
  testProviderModel,
} from '../../../../services/api/aiSettingsApi';
import Alert from '../../../ui/Alert';
import Button from '../../../ui/Button';
import Checkbox from '../../../ui/Checkbox';
import Label from '../../../ui/Label';
import { ModalShell } from '../../../ui/ModalShell';
import {
  appendTemperatureToProviderString,
  CLAUDE_CODE_DEFAULT_MODEL,
  type CloudProvider,
  type CustomDialogSource,
  type OllamaModel,
  type ProviderRef,
  type Workload,
  WORKLOAD_MODEL_HINT_KEYS,
} from './aiPanelTypes';
import { ModelTestResultPanel } from './ModelTestResultPanel';
import { ProviderModelPickerDialog } from './ProviderModelPickerDialog';
import { TemperatureOverrideField } from './TemperatureOverrideField';

export interface CustomRoutingDialogProps {
  workload: Workload;
  initial: ProviderRef;
  cloudProviders: CloudProvider[];
  localModels: OllamaModel[];
  ollamaRunning: boolean;
  /** Current per-model vision registry, used to prefill the vision checkbox. */
  modelRegistry: ModelRegistryEntry[];
  onClose: () => void;
  /** Emits the chosen provider ref plus the user's vision flag for that model. */
  onSubmit: (next: ProviderRef, vision: boolean) => void;
}

export const CustomRoutingDialog = ({
  workload,
  initial,
  cloudProviders,
  localModels,
  ollamaRunning,
  modelRegistry,
  onClose,
  onSubmit,
}: CustomRoutingDialogProps) => {
  const { t } = useT();
  // Non-openhuman cloud providers + local-ollama (if available) are the
  // "Custom" options. OpenHuman is its own Managed path; Default serializes
  // to the backend's `cloud` sentinel. Claude Code is excluded here — it has
  // its own dedicated `claude-code:` select option, not a generic cloud one.
  const customCloud = cloudProviders.filter(
    p => p.slug !== 'openhuman' && p.slug !== 'claude-code'
  );
  const localAvailable = ollamaRunning && localModels.length > 0;
  // Claude Code CLI is offered as a routing source only when its peer chip is
  // enabled (a cloud_providers entry exists).
  const claudeCodeEnabled = cloudProviders.some(p => p.slug === 'claude-code');

  const initialSource: CustomDialogSource | null =
    initial.kind === 'cloud'
      ? { kind: 'cloud', providerSlug: initial.providerSlug }
      : initial.kind === 'local'
        ? { kind: 'local' }
        : initial.kind === 'claude-code'
          ? { kind: 'claude-code' }
          : // `default` / `openhuman` are the managed refs. They used to fall
            // through to the first configured provider, which quietly
            // preselected a different route than the workload was actually on.
            { kind: 'managed' };

  const [source, setSource] = useState<CustomDialogSource | null>(initialSource);
  const [model, setModel] = useState<string>(() => {
    if (initial.kind === 'cloud' || initial.kind === 'local' || initial.kind === 'claude-code')
      return initial.model;
    if (initialSource?.kind === 'claude-code') return CLAUDE_CODE_DEFAULT_MODEL;
    return localModels[0]?.id ?? '';
  });
  const [testBusy, setTestBusy] = useState(false);
  const [testReply, setTestReply] = useState<string | null>(null);
  const [testError, setTestError] = useState<string | null>(null);
  const [testStartedAt, setTestStartedAt] = useState<string | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const testRequestIdRef = useRef(0);
  // Optional temperature override for this workload. `null` = use provider/global default;
  // a finite number means "send `temperature: X` upstream for this workload only".
  const [temperature, setTemperature] = useState<number | null>(
    initial.kind === 'cloud' || initial.kind === 'local' || initial.kind === 'claude-code'
      ? (initial.temperature ?? null)
      : null
  );

  // Registry slug for the selected source — keys the per-model vision flag.
  // Cloud uses the provider slug; local → `ollama`; claude-code → `claude-code`.
  const registrySlug =
    source?.kind === 'cloud'
      ? source.providerSlug
      : source?.kind === 'local'
        ? 'ollama'
        : source?.kind === 'claude-code'
          ? 'claude-code'
          : null;

  // The Vision workload always feeds the multimodal `vision-v1` path, so any
  // model routed here is treated as image-capable regardless of the per-model
  // registry flag. Force the flag on and lock the checkbox for this workload.
  const visionLocked = workload.id === 'vision';

  // User-set vision flag for this (provider, model). Prefilled from the registry,
  // re-prefilled whenever the selected provider/model changes. Always on (and
  // not user-editable) for the Vision workload.
  const [vision, setVision] = useState<boolean>(() =>
    visionLocked
      ? true
      : registrySlug && model.trim()
        ? modelRegistryVision(modelRegistry, registrySlug, model.trim())
        : false
  );
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setVision(
      visionLocked
        ? true
        : registrySlug && model.trim()
          ? modelRegistryVision(modelRegistry, registrySlug, model.trim())
          : false
    );
    // modelRegistry is stable for the dialog's lifetime (prop doesn't change mid-open).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [registrySlug, model, visionLocked]);

  // Managed carries no model id — the product picks one per workload — so
  // requiring one would make it permanently unsavable.
  const isManagedSource = source?.kind === 'managed';
  const canSave = source !== null && (isManagedSource || model.trim().length > 0);
  // …and there is nothing to send a test prompt to: the route is resolved
  // server-side at request time, not from a provider string here.
  const canTest = canSave && !isManagedSource;

  const resetTestState = () => {
    testRequestIdRef.current += 1;
    setTestReply(null);
    setTestError(null);
    setTestStartedAt(null);
    setTestBusy(false);
  };

  // The test call must name the same slug the save will persist, so this
  // mirrors `registrySlug` above rather than assuming a non-cloud source is
  // local: a `claude-code` route used to be tested as `ollama:<model>`, which
  // asked Ollama for a model it has never heard of and reported the failure as
  // the claude-code provider's.
  const currentProviderString =
    source == null || source.kind === 'managed' || registrySlug == null
      ? null
      : appendTemperatureToProviderString(
          `${registrySlug}:${model.trim()}`,
          temperature == null || !Number.isFinite(temperature) ? null : temperature
        );

  const handleSave = () => {
    if (!source || !canSave) return;
    const temp = temperature == null || !Number.isFinite(temperature) ? null : temperature;
    if (source.kind === 'cloud') {
      onSubmit(
        {
          kind: 'cloud',
          providerSlug: source.providerSlug,
          model: model.trim(),
          temperature: temp,
        },
        vision
      );
    } else if (source.kind === 'claude-code') {
      onSubmit({ kind: 'claude-code', model: model.trim(), temperature: temp }, vision);
    } else if (source.kind === 'local') {
      onSubmit({ kind: 'local', model: model.trim(), temperature: temp }, vision);
    } else {
      // Managed. `default` is the backend's managed sentinel; it takes no model
      // or temperature, so both are deliberately dropped rather than carried
      // over from whatever was selected before.
      onSubmit({ kind: 'default' }, vision);
    }
  };

  const handleTest = async () => {
    if (!currentProviderString || !canTest) return;
    const requestId = testRequestIdRef.current + 1;
    testRequestIdRef.current = requestId;
    setTestBusy(true);
    setTestReply(null);
    setTestError(null);
    setTestStartedAt(new Date().toLocaleTimeString());
    try {
      const result = await testProviderModel(workload.id, currentProviderString, 'Hello world');
      if (testRequestIdRef.current !== requestId) return;
      setTestReply(result.reply);
    } catch (err) {
      if (testRequestIdRef.current !== requestId) return;
      // #5146 §2.4: a raw upstream string ("401", "model_not_found", a bare
      // 404) tells the user nothing about what to change. Map the common
      // shapes onto a concrete next step; unrecognised errors pass through.
      const raw = err instanceof Error ? err.message : String(err);
      // The banner copy is deliberately generic (a provider error can echo
      // request material), so keep the raw text on the console where it is
      // still reachable for diagnosis.
      console.error(`[ai-settings][test] provider test failed workload=${workload.id}`, raw);
      // The bare slug, not `currentProviderString` — that is the composite
      // `provider:model[@temp]` and would read as "'openai:gpt-4o' rejected it".
      setTestError(describeProviderVerificationFailure(registrySlug ?? '', raw, t));
    } finally {
      if (testRequestIdRef.current === requestId) {
        setTestBusy(false);
      }
    }
  };

  // Nothing to route to *of the user's own*: no custom cloud providers, no
  // local Ollama, and the Claude Code peer chip is off. The warning still
  // explains that, but it no longer replaces the picker — managed is always
  // available, so a user with nothing configured must still be able to route a
  // workload back to it.
  const noOwnProviders = customCloud.length === 0 && !localAvailable && !claudeCodeEnabled;
  const selectedProviderLabel =
    source?.kind === 'cloud'
      ? (customCloud.find(provider => provider.slug === source.providerSlug)?.label ??
        source.providerSlug)
      : source?.kind === 'local'
        ? t('settings.ai.localOllama')
        : source?.kind === 'claude-code'
          ? t('settings.ai.claudeCode.modalTitle')
          : t('settings.ai.providerLabel');

  return (
    <ModalShell
      titleId="workload-routing-dialog-title"
      title={t('settings.ai.customRouting')}
      subtitle={<span id="workload-routing-dialog-subtitle">{t(workload.labelKey)}</span>}
      // The dialog used to name itself "Custom routing for <workload>"; the
      // title alone drops the workload, so keep the subtitle in the name.
      labelledBy="workload-routing-dialog-title workload-routing-dialog-subtitle"
      onClose={onClose}
      footer={
        <div className="flex justify-end gap-2">
          <Button type="button" variant="secondary" size="sm" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => void handleTest()}
            disabled={!canTest || testBusy}>
            {testBusy ? t('settings.ai.testing') : t('settings.ai.test')}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={handleSave}
            disabled={!canSave}>
            {t('common.save')}
          </Button>
        </div>
      }>
      <p className="mt-2 text-xs leading-5 text-content-muted">
        {t(WORKLOAD_MODEL_HINT_KEYS[workload.id])}
      </p>
      <div className="flex flex-col gap-4">
        {noOwnProviders && (
          <Alert variant="warning" className="p-3 text-xs">
            {t('settings.ai.noCustomProviders')}
          </Alert>
        )}
        <Button
          type="button"
          variant="secondary"
          size="md"
          onClick={() => setPickerOpen(true)}
          className="h-auto w-full justify-between px-3 py-2.5 text-left">
          <span className="flex min-w-0 flex-col gap-0.5">
            <span className="text-xs font-medium text-content-secondary">Provider and model</span>
            <span className="truncate text-sm font-medium text-content">
              {model ? `${selectedProviderLabel} · ${model}` : 'Select provider and model'}
            </span>
          </span>
          <span className="text-xs text-content-muted">Change</span>
        </Button>

        <TemperatureOverrideField temperature={temperature} onChange={setTemperature} />

        {/* Vision capability (optional). Marks a custom/BYOK model as
                accepting image input so the chat composer offers image
                attachments for it. Only shown once a concrete model is chosen. */}
        {registrySlug && model.trim().length > 0 && (
          <div className="flex flex-col gap-1.5">
            <Label className="inline-flex items-center gap-2 text-xs text-content-secondary">
              <Checkbox
                checked={visionLocked ? true : vision}
                onCheckedChange={setVision}
                disabled={visionLocked}
                className="h-3.5 w-3.5 disabled:opacity-60"
              />
              {t('settings.ai.modelVision')}
            </Label>
            <p className="text-[11px] text-content-faint">{t('settings.ai.modelVisionDesc')}</p>
          </div>
        )}

        <ModelTestResultPanel
          testBusy={testBusy}
          testReply={testReply}
          testError={testError}
          testStartedAt={testStartedAt}
          currentProviderString={currentProviderString}
        />

        {pickerOpen && (
          <ProviderModelPickerDialog
            cloudProviders={customCloud}
            localModels={localModels}
            ollamaRunning={ollamaRunning}
            claudeCodeEnabled={claudeCodeEnabled}
            initial={source && model ? { source, model } : null}
            onClose={() => setPickerOpen(false)}
            onSelect={({ source: nextSource, model: nextModel }) => {
              resetTestState();
              setSource(nextSource);
              setModel(nextModel);
              setPickerOpen(false);
            }}
          />
        )}
      </div>
    </ModalShell>
  );
};

export default CustomRoutingDialog;
