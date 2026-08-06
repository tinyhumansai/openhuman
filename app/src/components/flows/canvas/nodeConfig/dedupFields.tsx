/**
 * `dedup` node config form (issue #5263 — 14th tinyflows `NodeKind`). Skips
 * items already seen, keyed by a stable per-item `=`-expression evaluated
 * against each item (e.g. `=item.id`) — the engine tracks which keys have
 * already passed through this node and drops repeats.
 *
 * Field keys mirror what the engine actually reads at runtime:
 *  - `key` — `=`-bindable, required. The only config field this node has.
 */
import { useT } from '../../../../lib/i18n/I18nContext';
import { configString, ExpressionField } from './nodeConfigFields';
import type { UpstreamExpressionOption } from './upstreamOptions';

/**
 * Deliberately narrower than `NodeConfigFormProps` (`nodeConfigForms.tsx`) —
 * this form needs no `connections` (dedup nodes don't use the credential
 * picker), so it isn't imported here just to keep the shape identical. A
 * component typed against this subset is still assignable into
 * `NODE_CONFIG_FORMS`'s `NodeConfigForm` slot (the fuller prop type has
 * every property this one requires).
 */
export interface DedupFormProps {
  config: Record<string, unknown>;
  onChange: (patch: Record<string, unknown>) => void;
  upstreamOptions?: UpstreamExpressionOption[];
}

export function DedupForm({ config, onChange, upstreamOptions }: DedupFormProps) {
  const { t } = useT();

  return (
    <div className="space-y-3">
      <ExpressionField
        label={t('flows.nodeConfig.dedup.keyLabel')}
        hint={t('flows.nodeConfig.dedup.keyHint')}
        value={configString(config, 'key')}
        onChange={v => onChange({ key: v })}
        placeholder="=item.id"
        upstreamOptions={upstreamOptions}
        testId="node-config-dedup-key"
      />
    </div>
  );
}
