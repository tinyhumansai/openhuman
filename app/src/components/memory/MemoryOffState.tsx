/**
 * What every non-engine tab shows while memory is off (no usable engine, or
 * disabled on purpose): why, and a way to the Engine chip to change it.
 */
import { LuBrain } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import EmptyStateCard from '../EmptyStateCard';

interface MemoryOffStateProps {
  /** Engine-reported reason, when it gave one. */
  reason?: string | null;
  /** Memory was turned off on purpose (the Disabled option). */
  disabled?: boolean;
  onOpenEngine: () => void;
}

export default function MemoryOffState({ reason, disabled, onOpenEngine }: MemoryOffStateProps) {
  const { t } = useT();
  return (
    <div data-testid="memory-off-state">
      <EmptyStateCard
        icon={<LuBrain className="h-6 w-6 text-primary-500" aria-hidden />}
        title={disabled ? t('memoryPage.disabled.title') : t('memoryPage.off.title')}
        description={
          disabled
            ? t('memoryPage.disabled.description')
            : reason || t('memoryPage.off.description')
        }
        actionLabel={t('memoryPage.off.action')}
        onAction={onOpenEngine}
        actionTestId="memory-off-open-engine"
      />
    </div>
  );
}
