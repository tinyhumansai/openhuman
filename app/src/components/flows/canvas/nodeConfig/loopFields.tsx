/**
 * `loop` node config form (15th tinyflows `NodeKind`). A bounded loop head:
 * it emits its input on the `body` port until either its iteration cap or its
 * optional condition says stop, then emits on `done`. The loop itself is drawn
 * on the canvas — wiring the body's last node back to this one is what makes
 * the section repeat.
 *
 * Field keys mirror what the engine actually reads at runtime:
 *  - `max_iterations` — required, positive. How many passes the body may run.
 *  - `on_exceeded` — `error` (default, fails the run naming this node) or
 *    `continue` (stop looping and leave through `done` with partial results).
 *  - `condition` — optional `=`-expression. While truthy the loop continues;
 *    the first falsey result exits without consuming an iteration.
 */
import { useT } from '../../../../lib/i18n/I18nContext';
import {
  configNumber,
  configString,
  ExpressionField,
  NumberField,
  SelectField,
} from './nodeConfigFields';
import type { UpstreamExpressionOption } from './upstreamOptions';

/**
 * Deliberately narrower than `NodeConfigFormProps` (`nodeConfigForms.tsx`) —
 * a loop node uses no credential picker, so `connections` is not imported here
 * just to keep the shape identical. A component typed against this subset is
 * still assignable into `NODE_CONFIG_FORMS`'s `NodeConfigForm` slot.
 */
export interface LoopFormProps {
  config: Record<string, unknown>;
  onChange: (patch: Record<string, unknown>) => void;
  upstreamOptions?: UpstreamExpressionOption[];
}

export function LoopForm({ config, onChange, upstreamOptions }: LoopFormProps) {
  const { t } = useT();
  // The engine's own default when the key is absent, shown so the field is
  // never blank and the effective bound is always visible.
  const maxIterations = configNumber(config, 'max_iterations') ?? 25;
  const onExceeded = configString(config, 'on_exceeded') || 'error';

  return (
    <div className="space-y-3">
      <NumberField
        label={t('flows.nodeConfig.loop.maxIterationsLabel')}
        hint={t('flows.nodeConfig.loop.maxIterationsHint')}
        value={maxIterations}
        // The engine's domain is a positive integer, so the control is held to
        // the same one: without these the spinner walks into 0 and negatives,
        // and the editor happily builds a graph the core then refuses to save.
        min={1}
        step={1}
        onChange={v => onChange({ max_iterations: v })}
        testId="node-config-loop-max-iterations"
      />
      <SelectField
        label={t('flows.nodeConfig.loop.onExceededLabel')}
        hint={t('flows.nodeConfig.loop.onExceededHint')}
        value={onExceeded}
        onChange={v => onChange({ on_exceeded: v })}
        testId="node-config-loop-on-exceeded"
        options={[
          { value: 'error', label: t('flows.nodeConfig.loop.onExceeded_error') },
          { value: 'continue', label: t('flows.nodeConfig.loop.onExceeded_continue') },
        ]}
      />
      <ExpressionField
        label={t('flows.nodeConfig.loop.conditionLabel')}
        hint={t('flows.nodeConfig.loop.conditionHint')}
        value={configString(config, 'condition')}
        onChange={v => onChange({ condition: v })}
        placeholder="=item.needs_another_pass"
        upstreamOptions={upstreamOptions}
        testId="node-config-loop-condition"
      />
    </div>
  );
}
