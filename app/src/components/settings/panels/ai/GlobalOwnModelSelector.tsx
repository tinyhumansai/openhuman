/*
 * "Use Your Own Models" card — a single provider+model applied to every
 * workload at once (the "own" routing mode).
 */
import { useEffect, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import {
  type ModelRegistryEntry,
  modelRegistryVision,
} from '../../../../services/api/aiSettingsApi';
import Alert from '../../../ui/Alert';
import Button from '../../../ui/Button';
import Card from '../../../ui/Card';
import Checkbox from '../../../ui/Checkbox';
import Label from '../../../ui/Label';
import {
  CLAUDE_CODE_DEFAULT_MODEL,
  type CloudProvider,
  type CustomDialogSource,
  type OllamaModel,
  type ProviderRef,
  providerRefSignature,
  slugTone,
} from './aiPanelTypes';
import { ProviderSwatch } from './ProviderListRow';
import { ProviderModelPickerDialog } from './ProviderModelPickerDialog';

export const GlobalOwnModelSelector = ({
  current,
  saved,
  cloudProviders,
  localModels,
  ollamaRunning,
  modelRegistry,
  onApply,
}: {
  current: ProviderRef | null;
  saved: ProviderRef | null;
  cloudProviders: CloudProvider[];
  localModels: OllamaModel[];
  ollamaRunning: boolean;
  modelRegistry: ModelRegistryEntry[];
  onApply: (next: ProviderRef, vision: boolean) => Promise<void>;
}) => {
  const { t } = useT();
  // Claude Code is excluded from the generic cloud list — it has its own
  // dedicated `claude-code:` option below (offered when connected).
  const customCloud = cloudProviders.filter(
    p => p.slug !== 'openhuman' && p.slug !== 'claude-code'
  );
  const localAvailable = ollamaRunning && localModels.length > 0;
  const claudeCodeEnabled = cloudProviders.some(p => p.slug === 'claude-code');

  const initialSource: CustomDialogSource | null =
    current?.kind === 'cloud'
      ? { kind: 'cloud', providerSlug: current.providerSlug }
      : current?.kind === 'local'
        ? { kind: 'local' }
        : current?.kind === 'claude-code'
          ? { kind: 'claude-code' }
          : customCloud[0]
            ? { kind: 'cloud', providerSlug: customCloud[0].slug }
            : localAvailable
              ? { kind: 'local' }
              : claudeCodeEnabled
                ? { kind: 'claude-code' }
                : null;

  const [source, setSource] = useState<CustomDialogSource | null>(initialSource);
  const [model, setModel] = useState<string>(() => {
    if (current?.kind === 'cloud' || current?.kind === 'local' || current?.kind === 'claude-code') {
      return current.model;
    }
    if (initialSource?.kind === 'claude-code') return CLAUDE_CODE_DEFAULT_MODEL;
    return '';
  });
  // Registry slug for the selected source — keys the per-model vision flag.
  const registrySlug =
    source?.kind === 'cloud'
      ? source.providerSlug
      : source?.kind === 'local'
        ? 'ollama'
        : source?.kind === 'claude-code'
          ? 'claude-code'
          : null;
  const [vision, setVision] = useState<boolean>(() =>
    registrySlug && model.trim()
      ? modelRegistryVision(modelRegistry, registrySlug, model.trim())
      : false
  );
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setVision(
      registrySlug && model.trim()
        ? modelRegistryVision(modelRegistry, registrySlug, model.trim())
        : false
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [registrySlug, model]);
  const [saving, setSaving] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);

  const canApply = source !== null && model.trim().length > 0;
  const selectedRef =
    !source || !model.trim()
      ? null
      : source.kind === 'local'
        ? ({ kind: 'local', model: model.trim() } as const)
        : source.kind === 'claude-code'
          ? ({ kind: 'claude-code', model: model.trim() } as const)
          : // Managed is not offered by this card (`allowManaged={false}`), so
            // it can never be the source here; narrow rather than assume.
            source.kind === 'cloud'
            ? ({ kind: 'cloud', providerSlug: source.providerSlug, model: model.trim() } as const)
            : null;
  const isSaved =
    selectedRef !== null &&
    saved !== null &&
    providerRefSignature(selectedRef) === providerRefSignature(saved);
  const selectedProviderLabel =
    source?.kind === 'cloud'
      ? (customCloud.find(provider => provider.slug === source.providerSlug)?.label ??
        source.providerSlug)
      : source?.kind === 'local'
        ? t('settings.ai.provider.ollama')
        : source?.kind === 'claude-code'
          ? t('settings.ai.claudeCode.modalTitle')
          : t('settings.ai.globalModel.provider');
  const selectedProviderSlug =
    source?.kind === 'cloud'
      ? source.providerSlug
      : source?.kind === 'local'
        ? 'ollama'
        : source?.kind === 'claude-code'
          ? 'claude-code'
          : null;

  const applySelection = async (nextSource: CustomDialogSource | null, nextModel: string) => {
    if (!nextSource || !nextModel.trim()) return;
    setSaving(true);
    try {
      if (nextSource.kind === 'local') {
        await onApply({ kind: 'local', model: nextModel.trim() }, vision);
      } else if (nextSource.kind === 'claude-code') {
        await onApply({ kind: 'claude-code', model: nextModel.trim() }, vision);
      } else if (nextSource.kind === 'cloud') {
        await onApply(
          { kind: 'cloud', providerSlug: nextSource.providerSlug, model: nextModel.trim() },
          vision
        );
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card
      title={t('settings.ai.globalModel.title')}
      description={t('settings.ai.globalModel.desc')}
      className="w-full">
      <div className="flex flex-col gap-4 p-4">
        {customCloud.length === 0 && !localAvailable && !claudeCodeEnabled ? (
          <Alert variant="warning" className="p-3 text-xs">
            {t('settings.ai.globalModel.noProviders')}
          </Alert>
        ) : (
          <>
            <Button
              type="button"
              variant="secondary"
              size="md"
              onClick={() => setPickerOpen(true)}
              className="h-auto w-full justify-start gap-3 px-3 py-2.5 text-left">
              {selectedProviderSlug ? (
                <ProviderSwatch
                  slug={selectedProviderSlug}
                  label={selectedProviderLabel}
                  tone={slugTone(selectedProviderSlug)}
                />
              ) : null}
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="text-xs font-medium text-content-secondary">
                  Provider and model
                </span>
                <span className="truncate text-sm font-medium text-content">
                  {model ? `${selectedProviderLabel} · ${model}` : 'Select provider and model'}
                </span>
              </span>
              <span className="text-xs text-content-muted">Change</span>
            </Button>
            {registrySlug && model.trim().length > 0 && (
              <Label className="flex items-start gap-2 text-xs text-content-secondary">
                <Checkbox
                  checked={vision}
                  onCheckedChange={setVision}
                  className="mt-0.5 h-3.5 w-3.5"
                />
                <span>
                  {t('settings.ai.modelVision')}
                  <span className="block font-normal text-[11px] text-content-faint">
                    {t('settings.ai.modelVisionDesc')}
                  </span>
                </span>
              </Label>
            )}

            <Alert className="px-3 py-2 text-xs text-content-muted">
              {t('settings.ai.globalModel.appliesToAll')}
            </Alert>

            <div className="flex justify-end">
              <Button
                type="button"
                variant="primary"
                size="xs"
                disabled={!canApply || saving || isSaved}
                onClick={() => void applySelection(source, model)}>
                {saving
                  ? t('settings.ai.globalModel.saving')
                  : isSaved
                    ? t('settings.ai.globalModel.saved')
                    : t('common.save')}
              </Button>
            </div>
          </>
        )}
      </div>
      {pickerOpen && (
        <ProviderModelPickerDialog
          // This card IS the "own models" routing mode. Offering managed here
          // would let a click inside it flip the mode out from under the card,
          // so it is the one surface that opts out.
          allowManaged={false}
          cloudProviders={customCloud}
          localModels={localModels}
          ollamaRunning={ollamaRunning}
          claudeCodeEnabled={claudeCodeEnabled}
          initial={source && model ? { source, model } : null}
          onClose={() => setPickerOpen(false)}
          onSelect={({ source: nextSource, model: nextModel }) => {
            setSource(nextSource);
            setModel(nextModel);
            setPickerOpen(false);
          }}
        />
      )}
    </Card>
  );
};

export default GlobalOwnModelSelector;
