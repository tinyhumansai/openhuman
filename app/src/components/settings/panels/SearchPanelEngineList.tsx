import type { SearchEngineId } from '../../../utils/tauriCommands/config';
import Badge from '../../ui/Badge';
import { CheckIcon } from '../../ui/icons';
import { ToggleGroupItem, ToggleGroupRoot } from '../../ui/ToggleGroup';

export interface EngineOption {
  id: SearchEngineId;
  label: string;
  description: string;
  requiresKey: boolean;
}

interface SearchPanelEngineListProps {
  engines: EngineOption[];
  selectedEngine: SearchEngineId;
  ariaLabel: string;
  isConfigured: (engine: SearchEngineId) => boolean;
  onSelect: (engine: SearchEngineId) => void;
  t: (key: string) => string;
}

/**
 * The search-engine picker. A single-select `ToggleGroup` gives the real
 * `role="radiogroup"` / `role="radio"` / `aria-checked` pairing for free
 * (Radix sets these itself for `type="single"`), plus roving-focus arrow-key
 * navigation — the hand-rolled `<button role="radio">` list this replaced had
 * neither.
 */
const SearchPanelEngineList = ({
  engines,
  selectedEngine,
  ariaLabel,
  isConfigured,
  onSelect,
  t,
}: SearchPanelEngineListProps) => (
  <ToggleGroupRoot
    type="single"
    orientation="vertical"
    aria-label={ariaLabel}
    value={selectedEngine}
    onValueChange={value => {
      if (value) onSelect(value as SearchEngineId);
    }}
    className="flex w-full flex-col gap-0 divide-y divide-line-subtle rounded-xl border border-line bg-surface overflow-hidden">
    {engines.map(opt => {
      const selected = opt.id === selectedEngine;
      const configured = isConfigured(opt.id);
      const blocked = opt.requiresKey && !configured && selected;
      return (
        <ToggleGroupItem
          key={opt.id}
          value={opt.id}
          data-testid={`search-engine-${opt.id}`}
          variant="tertiary"
          className="w-full flex items-start justify-start gap-3 rounded-none border-0 px-4 py-3 h-auto text-left data-[state=on]:bg-primary-50 dark:data-[state=on]:bg-primary-500/10">
          <span className="flex-1 min-w-0">
            <span className="flex items-center gap-2">
              <span className="text-sm font-medium text-content">{opt.label}</span>
              {opt.requiresKey && (
                <Badge variant={configured ? 'success' : 'warning'}>
                  {configured
                    ? t('settings.search.statusConfigured')
                    : t('settings.search.statusNeedsKey')}
                </Badge>
              )}
            </span>
            <span className="block mt-0.5 text-xs text-content-muted">{opt.description}</span>
            {blocked && (
              <span className="block mt-1 text-[11px] text-amber-700 dark:text-amber-300">
                {t('settings.search.fallbackToManaged')}
              </span>
            )}
          </span>
          {selected && (
            <CheckIcon className="w-5 h-5 text-primary-500 shrink-0 mt-0.5" aria-hidden="true" />
          )}
        </ToggleGroupItem>
      );
    })}
  </ToggleGroupRoot>
);

export default SearchPanelEngineList;
