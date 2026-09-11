/*
 * Optional per-workload temperature override control — a checkbox that
 * reveals a Radix Slider + numeric field once enabled.
 */
import { useT } from '../../../../lib/i18n/I18nContext';
import Checkbox from '../../../ui/Checkbox';
import Label from '../../../ui/Label';
import NumberField from '../../../ui/NumberField';
import Slider from '../../../ui/Slider';

export const TemperatureOverrideField = ({
  temperature,
  onChange,
}: {
  temperature: number | null;
  onChange: (next: number | null) => void;
}) => {
  const { t } = useT();
  return (
    <div className="flex flex-col gap-1.5">
      <Label className="flex items-center justify-between gap-2 text-xs text-content-secondary">
        <span className="inline-flex items-center gap-2">
          <Checkbox
            checked={temperature != null}
            onCheckedChange={next => onChange(next ? 0.7 : null)}
            className="h-3.5 w-3.5"
          />
          {t('settings.ai.temperatureOverride')}
        </span>
        {temperature != null && (
          <span className="font-mono text-[11px] text-content-muted">{temperature.toFixed(2)}</span>
        )}
      </Label>
      {temperature != null && (
        <div className="flex items-center gap-2">
          <Slider
            value={[temperature]}
            min={0}
            max={2}
            step={0.05}
            thumbLabels={[t('settings.ai.temperatureOverrideSlider')]}
            onValueChange={vals => {
              const next = vals[0];
              if (next != null) onChange(next);
            }}
            className="flex-1"
          />
          <NumberField
            id="temperature-override-value"
            aria-label={t('settings.ai.temperatureOverrideValue')}
            value={String(temperature)}
            min={0}
            max={2}
            step={0.05}
            onChange={v => {
              const parsed = Number(v);
              if (Number.isFinite(parsed)) {
                onChange(Math.max(0, Math.min(2, parsed)));
              }
            }}
            onCommit={() => {}}
          />
        </div>
      )}
      <p className="text-[11px] text-content-faint">{t('settings.ai.temperatureOverrideDesc')}</p>
    </div>
  );
};

export default TemperatureOverrideField;
