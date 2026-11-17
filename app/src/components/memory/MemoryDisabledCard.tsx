import { PowerOff } from 'lucide-react';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import { Badge, Button } from '../ui';

export interface MemoryDisabledCardProps {
  /** Memory is disabled right now (the `none` engine is selected). */
  active: boolean;
  /** A switch is in flight; the button waits. */
  saving: boolean;
  onDisable: () => void;
}

/**
 * Memory → Provider: the "Disabled" option, beside the CortexDB card. It turns
 * memory off completely; each engine's endpoint and key are kept, so picking a
 * provider again turns it back on with no re-entry.
 */
export default function MemoryDisabledCard({ active, saving, onDisable }: MemoryDisabledCardProps) {
  const { t } = useT();
  return (
    <div
      className={cn(
        'flex items-start gap-2.5 rounded-xl border bg-surface px-3 py-2.5',
        active ? 'border-primary-500/60' : 'border-line'
      )}
      data-testid="memory-engine-disabled">
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-surface-strong ring-1 ring-line">
        <PowerOff className="h-4 w-4 text-content-secondary" aria-hidden />
      </span>
      <div className="min-w-0 flex-1 space-y-2">
        <div>
          <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1">
            <h4 className="text-sm font-semibold text-content">
              {t('memoryPage.engine.disabled.title')}
            </h4>
            {active && (
              <Badge variant="primary" data-testid="memory-engine-disabled-active">
                {t('memoryPage.engine.inUse')}
              </Badge>
            )}
          </div>
          <p className="text-xs leading-snug text-content-muted">
            {t('memoryPage.engine.disabled.description')}
          </p>
        </div>
        {!active && (
          <Button
            size="xs"
            variant="secondary"
            analyticsId="memory-engine-disable"
            data-testid="memory-engine-disable"
            disabled={saving}
            onClick={onDisable}>
            {t('memoryPage.engine.disabled.action')}
          </Button>
        )}
      </div>
    </div>
  );
}
