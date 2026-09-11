/*
 * Top-level routing-mode picker. These options are a real radio group: one
 * routing mode is always active, and the primitive supplies its keyboard and
 * screen-reader behavior.
 */
import { cn } from '../../../../lib/cn';
import { useT } from '../../../../lib/i18n/I18nContext';
import Card from '../../../ui/Card';
import Label from '../../../ui/Label';
import { RadioGroupItem, RadioGroupRoot } from '../../../ui/RadioGroup';
import type { RoutingMode } from './aiPanelTypes';

/** One cell of the segmented control. Selected-ness comes from the resolved
 *  mode rather than a `:has()` selector, so it works on the macOS 12 floor. */
const ModeOption = ({
  value,
  selected,
  title,
  description,
}: {
  value: RoutingMode;
  selected: boolean;
  title: string;
  description: string;
}) => (
  <Label
    data-slot="routing-mode-option"
    data-selected={selected}
    className={cn(
      'flex cursor-pointer items-center gap-3 rounded-lg px-3 py-2.5 transition-colors',
      selected ? 'bg-surface-muted' : 'hover:bg-surface-hover'
    )}>
    <RadioGroupItem value={value} size="md" className="flex-none" />
    <span className="flex min-w-0 flex-col gap-0.5">
      <span className="text-sm font-medium text-content">{title}</span>
      <span className="text-[11px] leading-4 text-content-muted">{description}</span>
    </span>
  </Label>
);

export const RoutingModeCards = ({
  effectiveRoutingMode,
  onSelectManaged,
  onSelectOwn,
  onSelectCustom,
}: {
  effectiveRoutingMode: RoutingMode;
  onSelectManaged: () => void;
  onSelectOwn: () => void;
  onSelectCustom: () => void;
}) => {
  const { t } = useT();
  return (
    <Card className="w-full">
      <div className="flex flex-col gap-3 p-4">
        <RadioGroupRoot
          aria-label={t('settings.ai.routing')}
          value={effectiveRoutingMode}
          onValueChange={next => {
            if (next === 'managed') onSelectManaged();
            else if (next === 'own') onSelectOwn();
            else if (next === 'custom') onSelectCustom();
          }}
          className="grid w-full gap-1">
          <ModeOption
            value="managed"
            selected={effectiveRoutingMode === 'managed'}
            title={t('settings.ai.routing.managed')}
            description={t('settings.ai.routing.managedDesc')}
          />
          <ModeOption
            value="own"
            selected={effectiveRoutingMode === 'own'}
            title={t('settings.ai.routing.useYourOwn')}
            description={t('settings.ai.routing.useYourOwnDesc')}
          />
          <ModeOption
            value="custom"
            selected={effectiveRoutingMode === 'custom'}
            title={t('settings.ai.routing.advanced')}
            description={t('settings.ai.routing.advancedDesc')}
          />
        </RadioGroupRoot>
      </div>
    </Card>
  );
};

export default RoutingModeCards;
