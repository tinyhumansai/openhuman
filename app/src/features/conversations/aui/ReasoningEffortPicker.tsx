/**
 * The composer's thinking-level control. Sits beside the model trigger and
 * picks the reasoning effort every send asks for (`reasoning_effort` on
 * `openhuman.channel_web_chat`), which the core turns into the provider's
 * reasoning setting. `default` leaves the choice to the provider.
 *
 * The level is remembered per model (`runtime.reasoning_effort_by_model`), so
 * switching models brings back the level last used with that model. The gauge
 * beside the label sweeps with the level, after OpenClaw's effort pill.
 */
import { useT } from '../../../lib/i18n/I18nContext';

export const REASONING_EFFORTS = [
  'default',
  'none',
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
] as const;

export type ReasoningEffortChoice = (typeof REASONING_EFFORTS)[number];

/** Narrows a stored/config value to a known choice; anything else is `default`. */
export function toReasoningEffortChoice(value: string | null | undefined): ReasoningEffortChoice {
  const normalized = (value ?? '').trim().toLowerCase();
  if (normalized === 'off') return 'none';
  if (normalized === 'max') return 'xhigh';
  return (REASONING_EFFORTS as readonly string[]).includes(normalized)
    ? (normalized as ReasoningEffortChoice)
    : 'default';
}

/**
 * Needle angle for a level: -120° (off) to +120° (max). `default` points
 * straight up, since the provider decides.
 */
export function gaugeAngleFor(value: ReasoningEffortChoice): number {
  if (value === 'default') return 0;
  const levels = REASONING_EFFORTS.slice(1);
  const index = levels.indexOf(value);
  return -120 + (240 * index) / (levels.length - 1);
}

function ThinkingGauge({ value }: { value: ReasoningEffortChoice }) {
  return (
    <svg
      data-testid="composer-reasoning-gauge"
      className="pointer-events-none h-3.5 w-3.5 flex-none"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      aria-hidden="true">
      <path d="M4.5 18a9 9 0 1 1 15 0" />
      <line
        x1="12"
        y1="15"
        x2="12"
        y2="8"
        transform={`rotate(${gaugeAngleFor(value)} 12 15)`}
        className="transition-transform duration-200 motion-reduce:transition-none"
      />
      <circle cx="12" cy="15" r="1.25" fill="currentColor" stroke="none" />
    </svg>
  );
}

export function ReasoningEffortPicker({
  value,
  onChange,
  disabled = false,
  modelLabel,
}: {
  value: ReasoningEffortChoice;
  onChange: (value: ReasoningEffortChoice) => void;
  disabled?: boolean;
  /** The model this level is remembered for; named in the tooltip. */
  modelLabel?: string | null;
}) {
  const { t } = useT();
  const label = t('composer.reasoning.label');
  const title = modelLabel
    ? t('composer.reasoning.forModel').replace('{model}', modelLabel)
    : label;
  return (
    <span className="inline-flex h-7 min-w-0 items-center gap-1 rounded-md pl-2 text-content-muted transition-colors hover:bg-surface-hover hover:text-content">
      <ThinkingGauge value={value} />
      <select
        data-testid="composer-reasoning-effort"
        data-analytics-id="chat-reasoning-effort"
        aria-label={label}
        title={title}
        value={value}
        disabled={disabled}
        onChange={event => onChange(toReasoningEffortChoice(event.target.value))}
        className="h-7 min-w-0 cursor-pointer rounded-md border-none bg-transparent pr-2 text-xs font-medium text-inherit focus:outline-none focus-visible:ring-1 focus-visible:ring-line disabled:opacity-50">
        {REASONING_EFFORTS.map(effort => (
          <option key={effort} value={effort}>
            {t(`composer.reasoning.${effort}`)}
          </option>
        ))}
      </select>
    </span>
  );
}

export default ReasoningEffortPicker;
