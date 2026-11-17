import { KeyRound, Monitor } from 'lucide-react';
import type { ReactNode } from 'react';

import cortexdbLogo from '../../assets/provider-icons/cortexdb.png';
import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import ChipTabs from '../layout/ChipTabs';
import { Badge, type BadgeVariant } from '../ui';
import MemoryProviderLogo, { type MemoryProviderOption } from './MemoryProviderLogo';

/** The product name; a brand, so it is not translated. */
export const CORTEXDB = 'CortexDB';

export interface MemoryCortexCardProps {
  /** The connection whose details are showing. */
  selected: MemoryProviderOption;
  onSelect: (option: MemoryProviderOption) => void;
  /** The configured connection, if any (marked on its chip). */
  active: MemoryProviderOption | null;
  /** Status of the configured connection; null when nothing is configured. */
  status: { variant: BadgeVariant; label: string } | null;
  /** The selected connection's panel. */
  children: ReactNode;
}

/**
 * Memory → Provider: memory runs on CortexDB, so it is one CortexDB card, with
 * a chip per way to reach it — via TinyHumans (free), your own CortexDB API key,
 * or a server on this computer — like Gemini on Voice agents is one service
 * reached managed or with your own key.
 */
export default function MemoryCortexCard({
  selected,
  onSelect,
  active,
  status,
  children,
}: MemoryCortexCardProps) {
  const { t } = useT();

  const chip = (
    option: MemoryProviderOption,
    icon: ReactNode,
    label: string,
    extra?: ReactNode
  ) => (
    <span className="inline-flex items-center gap-1.5">
      {icon}
      {label}
      {extra}
      {option === active && (
        <span
          className="h-1.5 w-1.5 rounded-full bg-primary-500"
          aria-label={t('memoryPage.engine.inUse')}
          data-testid={`memory-engine-chip-active-${option}`}
        />
      )}
    </span>
  );

  return (
    <div
      className={cn(
        'overflow-hidden rounded-xl border bg-surface',
        active ? 'border-primary-500/60' : 'border-line'
      )}
      data-testid="memory-engines">
      <div className="flex items-center gap-2.5 px-3 py-2.5">
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-white ring-1 ring-line">
          <img src={cortexdbLogo} alt="" aria-hidden className="h-5 w-5 object-contain" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1">
            <h4 className="text-sm font-semibold text-content">{CORTEXDB}</h4>
            {status && (
              <Badge variant={status.variant} data-testid="memory-engine-status">
                {status.label}
              </Badge>
            )}
          </div>
          <p className="text-xs leading-snug text-content-muted">
            {t('memoryPage.engine.cortex.description')}
          </p>
        </div>
      </div>

      <div className="border-t border-line-subtle px-3 pt-2.5 pb-3">
        <ChipTabs<MemoryProviderOption>
          items={[
            {
              id: 'builtin',
              testId: 'memory-engine-builtin',
              label: chip(
                'builtin',
                <MemoryProviderLogo option="builtin" className="h-3.5 w-3.5" />,
                t('memoryPage.engine.chip.builtin'),
                <span className="rounded-full bg-sage-600 px-1.5 text-[10px] leading-4 font-semibold text-white">
                  {t('memoryPage.engine.chip.free')}
                </span>
              ),
            },
            {
              id: 'apikey',
              testId: 'memory-engine-apikey',
              label: chip(
                'apikey',
                <KeyRound className="h-3.5 w-3.5" aria-hidden />,
                t('memoryPage.engine.chip.apikey')
              ),
            },
            {
              id: 'selfhost',
              testId: 'memory-engine-selfhost',
              label: chip(
                'selfhost',
                <Monitor className="h-3.5 w-3.5" aria-hidden />,
                t('memoryPage.engine.chip.selfhost')
              ),
            },
          ]}
          value={selected}
          onChange={onSelect}
          ariaLabel={t('memoryPage.engine.chip.label')}
          className="flex flex-wrap gap-1.5 pb-2"
        />
        {children}
      </div>
    </div>
  );
}
