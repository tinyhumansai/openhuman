import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { loadAISettings } from '../../services/api/aiSettingsApi';
import { type CloudProvider } from '../settings/panels/ai/aiPanelTypes';
import {
  ProviderModelPickerDialog,
  type ProviderModelSelection,
} from '../settings/panels/ai/ProviderModelPickerDialog';
import { Button } from '../ui';

interface ModelQualityPillProps {
  className?: string;
  value?: string | null;
  /**
   * `null` means "managed" — the host clears its override and routing falls
   * back to whatever the product picks. It is deliberately not the empty
   * string: the host resolves its override with `??`, which passes `''`
   * straight through as a real (and empty) model id.
   */
  onValueChange?: (value: string | null, contextWindow?: number | null) => void;
}

/**
 * A managed passthrough model id: `openrouter/<author>/<slug>`, optionally with
 * a `:tag` variant suffix (`:free`, `:nitro`).
 *
 * These are encoded BARE (no `slug:` prefix) because the managed backend
 * addresses its own models directly. Both round-trip helpers below must special
 * case them: a BYOK value is `providerSlug:model` where the slug never contains
 * `/`, so splitting a passthrough id on `:` would parse
 * `openrouter/nex-agi/nex-n2.5-mini:free` as provider
 * `openrouter/nex-agi/nex-n2.5-mini` + model `free`, and one without any `:`
 * would decode to null and silently drop the selection on reopen.
 */
/**
 * Stable empty list. An inline `[]` prop is a new identity on every render,
 * which re-triggered the picker's catalog effect each render.
 */
const NO_LOCAL_MODELS: never[] = [];

function isManagedPassthroughId(value: string): boolean {
  if (!value.startsWith('openrouter/')) return false;
  const rest = value.slice('openrouter/'.length).split(':')[0];
  return rest.split('/').filter(Boolean).length === 2;
}

function selectionFromValue(value: string | null | undefined): ProviderModelSelection | null {
  if (!value || value.startsWith('hint:')) return null;
  if (isManagedPassthroughId(value)) {
    return { source: { kind: 'managed' }, model: value };
  }
  const separator = value.indexOf(':');
  if (separator <= 0 || separator === value.length - 1) return null;
  return {
    source: { kind: 'cloud', providerSlug: value.slice(0, separator) },
    model: value.slice(separator + 1),
  };
}

function selectionValue(selection: ProviderModelSelection): string | null {
  const { source, model } = selection;
  switch (source.kind) {
    // Managed with no model keeps the original contract: clearing the override
    // IS the choice, and the product routes per workload. A pinned catalog id
    // (e.g. `openrouter/deepseek/deepseek-v4-flash`) is sent bare — no
    // `slug:` prefix — because the managed backend addresses its own models
    // directly, and a prefix would be parsed as a BYOK provider selector.
    case 'managed':
      return model.trim() ? model.trim() : null;
    case 'cloud':
      return `${source.providerSlug}:${model}`;
    case 'local':
      return `ollama:${model}`;
    case 'claude-code':
      return `claude-code:${model}`;
  }
}

function displayValue(value: string | null | undefined): string {
  if (!value || value.startsWith('hint:')) return 'OpenHuman';
  // Managed ids carry no `providerSlug:` prefix to strip, and slicing on `:`
  // would reduce `…/nex-n2.5-mini:free` to just `free`. Show the model name.
  if (isManagedPassthroughId(value)) {
    const rest = value.slice('openrouter/'.length);
    return rest.split('/').slice(1).join('/');
  }
  const separator = value.indexOf(':');
  return separator >= 0 ? value.slice(separator + 1) : value;
}

/**
 * assistant-ui's compact model-selector trigger, backed by OpenHuman's shared
 * provider/model picker so configured providers and their model discovery stay
 * consistent with routing.
 */
export default function ModelQualityPill({
  className,
  value,
  onValueChange,
}: ModelQualityPillProps) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const [providers, setProviders] = useState<CloudProvider[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open || providers.length > 0) return;
    let active = true;
    setLoading(true);
    void loadAISettings()
      .then(settings => {
        if (!active) return;
        setProviders(
          settings.cloudProviders.map(provider => ({
            id: provider.id,
            slug: provider.slug,
            label: provider.label,
            endpoint: provider.endpoint,
            authStyle: provider.auth_style,
            maskedKey: provider.has_api_key ? '••••' : '',
          }))
        );
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [open, providers.length]);

  const initial = useMemo(() => selectionFromValue(value), [value]);

  return (
    <>
      <Button
        type="button"
        variant="tertiary"
        size="xs"
        analyticsId="chat-model-selector"
        aria-label={t('composer.modelSelector')}
        title={t('composer.modelSelector')}
        disabled={!onValueChange || loading}
        onClick={() => setOpen(true)}
        className={`h-7 min-w-0 rounded-md px-2 text-xs text-content-muted hover:bg-surface-hover hover:text-content ${className ?? ''}`}>
        <span className="min-w-0 truncate font-medium">
          {loading ? 'Loading models…' : displayValue(value)}
        </span>
        <svg
          className="ml-1 size-3.5 shrink-0 opacity-50"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
          aria-hidden>
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="m6 9 6 6 6-6" />
        </svg>
      </Button>
      {open && !loading && (
        <ProviderModelPickerDialog
          cloudProviders={providers}
          localModels={NO_LOCAL_MODELS}
          ollamaRunning={false}
          claudeCodeEnabled={false}
          initial={initial}
          onClose={() => setOpen(false)}
          onSelect={selection => {
            onValueChange?.(selectionValue(selection), selection.contextWindow);
            setOpen(false);
          }}
        />
      )}
    </>
  );
}
