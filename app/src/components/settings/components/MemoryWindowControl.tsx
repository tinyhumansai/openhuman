import { useEffect, useState } from 'react';

import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  isTauri,
  MEMORY_CONTEXT_WINDOWS,
  type MemoryContextWindow,
  openhumanGetConfig,
  openhumanUpdateMemorySettings,
} from '../../../utils/tauriCommands';
import Card from '../../ui/Card';
import { RadioGroupItem, RadioGroupRoot } from '../../ui/RadioGroup';

interface PresetMeta {
  label: string;
  badge: string;
  hint: string;
}

const isMemoryContextWindow = (value: unknown): value is MemoryContextWindow =>
  typeof value === 'string' && (MEMORY_CONTEXT_WINDOWS as readonly string[]).includes(value);

const extractCurrentWindow = (snapshot: unknown): MemoryContextWindow => {
  if (!snapshot || typeof snapshot !== 'object') return 'balanced';
  const root = snapshot as Record<string, unknown>;
  const config = (root.config as Record<string, unknown> | undefined) ?? root;
  const agent = config.agent as Record<string, unknown> | undefined;
  const candidate = agent?.memory_window;
  return isMemoryContextWindow(candidate) ? candidate : 'balanced';
};

interface Props {
  onError?: (message: string) => void;
  onSaved?: (window: MemoryContextWindow) => void;
}

/**
 * Stepped memory-context window selector.
 *
 * - Reads the persisted preference from the core via `openhuman.get_config`.
 * - Writes it back via `openhuman.update_memory_settings` (the core
 *   owns the actual char-budget mapping).
 * - Renders four options with plain-language hints so users understand
 *   the cost / continuity tradeoff.
 */
const MemoryWindowControl = ({ onError, onSaved }: Props) => {
  const { t } = useT();
  const [current, setCurrent] = useState<MemoryContextWindow>('balanced');
  const [pending, setPending] = useState<MemoryContextWindow | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState<MemoryContextWindow | null>(null);

  const localizedMeta: Record<MemoryContextWindow, PresetMeta> = {
    minimal: {
      label: t('settings.memoryWindow.minimal.label'),
      badge: t('settings.memoryWindow.minimal.badge'),
      hint: t('settings.memoryWindow.minimal.hint'),
    },
    balanced: {
      label: t('settings.memoryWindow.balanced.label'),
      badge: t('settings.memoryWindow.balanced.badge'),
      hint: t('settings.memoryWindow.balanced.hint'),
    },
    extended: {
      label: t('settings.memoryWindow.extended.label'),
      badge: t('settings.memoryWindow.extended.badge'),
      hint: t('settings.memoryWindow.extended.hint'),
    },
    maximum: {
      label: t('settings.memoryWindow.maximum.label'),
      badge: t('settings.memoryWindow.maximum.badge'),
      hint: t('settings.memoryWindow.maximum.hint'),
    },
  };

  useEffect(() => {
    if (!isTauri()) {
      setLoaded(true);
      return;
    }
    let cancelled = false;
    const load = async () => {
      try {
        const response = await openhumanGetConfig();
        if (cancelled) return;
        setCurrent(extractCurrentWindow(response.result));
      } catch (err) {
        if (cancelled) return;
        onError?.(err instanceof Error ? err.message : 'Failed to load memory settings');
      } finally {
        if (!cancelled) setLoaded(true);
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [onError]);

  const select = async (next: MemoryContextWindow) => {
    if (next === current || saving) return;
    setPending(next);
    setSaving(next);
    try {
      if (isTauri()) {
        await openhumanUpdateMemorySettings({ memory_window: next });
      }
      setCurrent(next);
      onSaved?.(next);
    } catch (err) {
      onError?.(err instanceof Error ? err.message : 'Failed to save memory window');
    } finally {
      setSaving(null);
      setPending(null);
    }
  };

  const activeForUi = pending ?? current;
  const meta = localizedMeta[activeForUi];

  return (
    <Card
      title={t('settings.memoryWindow.title')}
      description={t('settings.memoryWindow.description')}
      data-testid="memory-window-control">
      <div className="space-y-3 p-4">
        <RadioGroupRoot
          value={activeForUi}
          onValueChange={next => void select(next as MemoryContextWindow)}
          aria-label={t('settings.memoryWindow.title')}
          className="grid grid-cols-2 gap-2">
          {MEMORY_CONTEXT_WINDOWS.map(option => {
            const optionMeta = localizedMeta[option];
            const isActive = activeForUi === option;
            const isSaving = saving === option;
            const inputId = `memory-window-option-${option}-input`;
            return (
              <label
                key={option}
                htmlFor={inputId}
                className={cn(
                  'flex min-w-0 items-center justify-between gap-2 rounded-md border px-3 py-2 transition-colors',
                  isActive
                    ? 'border-primary-500 bg-primary-500/10'
                    : 'border-line-strong hover:bg-surface-hover',
                  (!loaded || (saving !== null && !isSaving)) && 'opacity-60 pointer-events-none'
                )}>
                <RadioGroupItem
                  id={inputId}
                  value={option}
                  data-testid={`memory-window-option-${option}`}
                  disabled={!loaded || (saving !== null && !isSaving)}
                  className="sr-only"
                />
                <span className="min-w-0 flex-1 truncate font-medium text-content">
                  {optionMeta.label}
                </span>
                <span className="shrink-0 whitespace-nowrap text-[10px] uppercase tracking-wide text-content-muted">
                  {optionMeta.badge}
                </span>
              </label>
            );
          })}
        </RadioGroupRoot>
        <p className="text-xs text-content-muted" data-testid="memory-window-hint">
          {meta.hint}
        </p>
      </div>
    </Card>
  );
};

export default MemoryWindowControl;
