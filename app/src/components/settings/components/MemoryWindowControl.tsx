import { useEffect, useState } from 'react';

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

/**
 * Plain-language framing for each preset. The actual character budgets
 * live in the Rust core (`MemoryContextWindow::limits` in
 * `src/openhuman/config/schema/agent.rs`) — these strings only describe
 * the UX tradeoff so users can pick without doing math.
 */
export const MEMORY_WINDOW_PRESET_META: Record<MemoryContextWindow, PresetMeta> = {
  minimal: {
    label: 'Minimal',
    badge: 'Cheapest',
    hint: 'Smallest memory window. Cheapest, fastest, least continuity between runs.',
  },
  balanced: {
    label: 'Balanced',
    badge: 'Recommended',
    hint: 'Sensible default — good continuity without burning extra tokens on every run.',
  },
  extended: {
    label: 'Extended',
    badge: 'More context',
    hint: 'More long-term memory injected into each run. Higher token cost per turn.',
  },
  maximum: {
    label: 'Maximum',
    badge: 'Highest cost',
    hint: 'The largest safe window. Best continuity, meaningfully higher token bill on every run.',
  },
};

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
  const [current, setCurrent] = useState<MemoryContextWindow>('balanced');
  const [pending, setPending] = useState<MemoryContextWindow | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState<MemoryContextWindow | null>(null);

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
  const meta = MEMORY_WINDOW_PRESET_META[activeForUi];

  return (
    <div
      className="border border-border rounded-lg p-4 space-y-3 bg-background"
      data-testid="memory-window-control">
      <div className="flex items-baseline justify-between">
        <div>
          <h3 className="text-base font-semibold">Long-term memory window</h3>
          <p className="text-sm text-muted-foreground">
            How much remembered context OpenHuman injects into every new agent run. Larger windows
            feel more aware of past conversations but use more tokens — and cost more — on every
            run.
          </p>
        </div>
      </div>
      <div
        role="radiogroup"
        aria-label="Long-term memory window"
        className="grid grid-cols-2 gap-2">
        {MEMORY_CONTEXT_WINDOWS.map(option => {
          const optionMeta = MEMORY_WINDOW_PRESET_META[option];
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
