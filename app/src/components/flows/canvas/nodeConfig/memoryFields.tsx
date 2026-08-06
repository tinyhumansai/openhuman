/**
 * `memory` node config form (issue #5226 — 13th tinyflows `NodeKind`).
 * Declarative, in-graph memory access: `recall`/`search`/`flavour`/`people`
 * read; `remember`/`forget` write. Spec: `my_docs/memory_access_in_workflows/08-memory-node.md`.
 *
 * Field keys mirror what the engine actually reads at runtime (per the
 * design doc's `MemoryProvider` trait + node config table):
 *  - `operation`  — always. `recall` · `search` · `flavour` · `people` ·
 *    `remember` · `forget`.
 *  - `scope`      — for `recall`/`search`/`remember`/`forget`. `user` and
 *    `flows` are read-only scopes (only offered for the read operations);
 *    `remember`/`forget` may only ever target `flow`.
 *  - `query`      — `=`-bindable, for `recall`/`search` (and optionally
 *    `people`, whose `people(query: Option<&str>)` signature takes one too).
 *  - `flavour`    — a plain slug (not an expression), for `flavour`.
 *  - `key`        — `=`-bindable, for `remember`/`forget`.
 *  - `value`      — `=`-bindable, for `remember` only.
 *  - `limit` / `min_score` — optional numeric caps on the read operations.
 *
 * Progressive disclosure mirrors the other multi-field forms in
 * `nodeConfigForms.tsx` (e.g. `TriggerForm`'s `trigger_kind` switch): only
 * the fields relevant to the selected `operation` render.
 */
import createDebug from 'debug';
import { useEffect } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import {
  configNumber,
  configString,
  ExpressionField,
  NumberField,
  SelectField,
} from './nodeConfigFields';
import type { UpstreamExpressionOption } from './upstreamOptions';

const log = createDebug('app:flows:nodeConfig:memory');

const MEMORY_OPERATIONS = ['recall', 'search', 'flavour', 'people', 'remember', 'forget'] as const;
type MemoryOperation = (typeof MEMORY_OPERATIONS)[number];

/**
 * The seven persona facets the `flavour` operation's `memory_flavour` engine
 * reader (`src/openhuman/memory/tools/flavour.rs`) accepts — any other slug
 * returns "Unknown flavour". Rendered as a dropdown rather than free text so
 * an author can't type an invalid slug (e.g. `email-tone`) in the first place.
 */
const MEMORY_FLAVOURS = [
  'communication',
  'coding_style',
  'stack',
  'workflow',
  'environment',
  'directives',
  'anti_preferences',
] as const;

/** Operations that read/write at a `scope`. `flavour` and `people` don't take one. */
const SCOPED_OPERATIONS = new Set<MemoryOperation>(['recall', 'search', 'remember', 'forget']);

/**
 * `remember`/`forget` are flow-only writes — the engine rejects a
 * `user`-scoped write at `validate_all` time (the hard security invariant
 * from `05-security.md`/`08-memory-node.md`). The UI must never let an
 * author construct that pair, so these operations get a `scope` control
 * offering only `flow`, never `user` (or the other read-only scope, `flows`).
 */
const WRITE_OPERATIONS = new Set<MemoryOperation>(['remember', 'forget']);

/** Operations that take a free-text `query`. */
const QUERY_OPERATIONS = new Set<MemoryOperation>(['recall', 'search', 'people']);

/** Operations whose result set can be capped/thresholded. */
const RESULT_OPERATIONS = new Set<MemoryOperation>(['recall', 'search', 'people']);

/**
 * Deliberately narrower than `NodeConfigFormProps` (`nodeConfigForms.tsx`) —
 * this form needs no `connections` (memory nodes don't use the credential
 * picker), so it isn't imported here just to keep the shape identical. A
 * component typed against this subset is still assignable into
 * `NODE_CONFIG_FORMS`'s `NodeConfigForm` slot (the fuller prop type has
 * every property this one requires).
 */
export interface MemoryFormProps {
  config: Record<string, unknown>;
  onChange: (patch: Record<string, unknown>) => void;
  upstreamOptions?: UpstreamExpressionOption[];
}

export function MemoryForm({ config, onChange, upstreamOptions }: MemoryFormProps) {
  const { t } = useT();
  const operation = (configString(config, 'operation') || 'recall') as MemoryOperation;
  const scope = configString(config, 'scope');
  const isWrite = WRITE_OPERATIONS.has(operation);

  // Self-heal: if this node arrives with an already-invalid `remember`/
  // `forget` + non-`flow` scope (a raw-JSON edit, an older draft, a
  // workflow-builder proposal), correct it as soon as the form mounts/updates
  // rather than only guarding the operation-change handler below — the
  // invariant must hold no matter how the bad pair got here.
  useEffect(() => {
    if (isWrite && scope !== 'flow') {
      log(
        'self-heal: clamping scope=%s to flow for write operation=%s',
        scope || '(empty)',
        operation
      );
      onChange({ scope: 'flow' });
    }
  }, [isWrite, scope, operation, onChange]);

  const handleOperationChange = (value: string) => {
    const next = value as MemoryOperation;
    const patch: Record<string, unknown> = { operation: next };
    // Hard UI rule mirroring the engine invariant: switching straight into a
    // write operation while a read-only scope (`user`/`flows`) is selected
    // must clamp to `flow` in the same patch, not rely on the effect above
    // (which would otherwise author one extra, momentarily-invalid patch).
    if (WRITE_OPERATIONS.has(next) && scope !== 'flow') {
      patch.scope = 'flow';
    }
    log('operation change: %s -> %s (scope patch=%s)', operation, next, patch.scope ?? 'none');
    onChange(patch);
  };

  const scopeOptions = isWrite
    ? [{ value: 'flow', label: t('flows.nodeConfig.memory.scope_flow') }]
    : [
        { value: 'user', label: t('flows.nodeConfig.memory.scope_user') },
        { value: 'flow', label: t('flows.nodeConfig.memory.scope_flow') },
        { value: 'flows', label: t('flows.nodeConfig.memory.scope_flows') },
      ];

  return (
    <div className="space-y-3">
      <SelectField
        label={t('flows.nodeConfig.memory.operationLabel')}
        value={operation}
        onChange={handleOperationChange}
        testId="node-config-memory-operation"
        options={MEMORY_OPERATIONS.map(op => ({
          value: op,
          label: t(`flows.nodeConfig.memory.operation_${op}`),
        }))}
      />

      {SCOPED_OPERATIONS.has(operation) && (
        <SelectField
          label={t('flows.nodeConfig.memory.scopeLabel')}
          hint={
            isWrite
              ? t('flows.nodeConfig.memory.scopeWriteHint')
              : t('flows.nodeConfig.memory.scopeHint')
          }
          value={isWrite ? 'flow' : scope || 'user'}
          onChange={v => onChange({ scope: v })}
          testId="node-config-memory-scope"
          options={scopeOptions}
        />
      )}

      {QUERY_OPERATIONS.has(operation) && (
        <ExpressionField
          label={t('flows.nodeConfig.memory.queryLabel')}
          hint={operation === 'people' ? t('flows.nodeConfig.memory.queryOptionalHint') : undefined}
          value={configString(config, 'query')}
          onChange={v => onChange({ query: v })}
          placeholder="=item.title"
          upstreamOptions={upstreamOptions}
          testId="node-config-memory-query"
        />
      )}

      {operation === 'flavour' && (
        <SelectField
          label={t('flows.nodeConfig.memory.flavourLabel')}
          hint={t('flows.nodeConfig.memory.flavourHint')}
          value={configString(config, 'flavour') || MEMORY_FLAVOURS[0]}
          onChange={v => onChange({ flavour: v })}
          testId="node-config-memory-flavour"
          options={MEMORY_FLAVOURS.map(facet => ({
            value: facet,
            label: t(`flows.nodeConfig.memory.flavour_${facet}`),
          }))}
        />
      )}

      {(operation === 'remember' || operation === 'forget') && (
        <ExpressionField
          label={t('flows.nodeConfig.memory.keyLabel')}
          value={configString(config, 'key')}
          onChange={v => onChange({ key: v })}
          placeholder="=item.id"
          upstreamOptions={upstreamOptions}
          testId="node-config-memory-key"
        />
      )}

      {operation === 'remember' && (
        <ExpressionField
          label={t('flows.nodeConfig.memory.valueLabel')}
          value={configString(config, 'value')}
          onChange={v => onChange({ value: v })}
          placeholder="=item"
          upstreamOptions={upstreamOptions}
          testId="node-config-memory-value"
        />
      )}

      {RESULT_OPERATIONS.has(operation) && (
        <div className="grid grid-cols-2 gap-3">
          <NumberField
            label={t('flows.nodeConfig.memory.limitLabel')}
            hint={t('flows.nodeConfig.memory.limitHint')}
            value={configNumber(config, 'limit')}
            onChange={v => onChange({ limit: v })}
            min={1}
            step={1}
            testId="node-config-memory-limit"
          />
          <NumberField
            label={t('flows.nodeConfig.memory.minScoreLabel')}
            hint={t('flows.nodeConfig.memory.minScoreHint')}
            value={configNumber(config, 'min_score')}
            onChange={v => onChange({ min_score: v })}
            min={0}
            max={1}
            step={0.05}
            testId="node-config-memory-min-score"
          />
        </div>
      )}
    </div>
  );
}
