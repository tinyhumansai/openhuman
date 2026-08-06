import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  isTauri,
  MEMORY_CONTEXT_WINDOWS,
  type MemoryContextWindow,
  openhumanGetConfig,
  openhumanUpdateMemorySettings,
} from '../../../utils/tauriCommands';

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
    <div
      className="border border-border rounded-lg p-4 space-y-3 bg-background"
      data-testid="memory-window-control">
      <div className="flex items-baseline justify-between">
        <div>
          <h3 className="text-base font-semibold text-content">
            {t('settings.memoryWindow.title')}
          </h3>
          <p className="text-sm text-muted-foreground">{t('settings.memoryWindow.description')}</p>
        </div>
      </div>
      <div
        role="radiogroup"
        aria-label={t('settings.memoryWindow.title')}
        className="grid grid-cols-2 gap-2">
        {MEMORY_CONTEXT_WINDOWS.map(option => {
          const optionMeta = localizedMeta[option];
          const isActive = activeForUi === option;
          const isSaving = saving === option;
          return (
            <button
              key={option}
              type="button"
              role="radio"
              aria-checked={isActive}
              data-testid={`memory-window-option-${option}`}
              disabled={!loaded || (saving !== null && !isSaving)}
              onClick={() => void select(option)}
              className={`text-left rounded-md border px-3 py-2 transition-colors ${
                isActive ? 'border-primary bg-primary/10' : 'border-border hover:bg-accent/40'
              } disabled:opacity-60 disabled:cursor-not-allowed`}>
              <div className="flex items-center justify-between gap-1 min-w-0">
                <span className="font-medium truncate">{optionMeta.label}</span>
                <span className="text-[10px] uppercase tracking-wide text-muted-foreground shrink-0 whitespace-nowrap">
                  {optionMeta.badge}
                </span>
              </div>
            </button>
          );
        })}
      </div>
      <p className="text-xs text-muted-foreground" data-testid="memory-window-hint">
        {meta.hint}
      </p>
    </div>
  );
};

export default MemoryWindowControl;
