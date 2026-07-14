/**
 * Shared, presentation-only field primitives for the node-config drawer
 * (issue B5b / Phase 3b). Each per-kind form (`nodeConfigForms.tsx`) composes
 * these; the drawer owns the controlled config object and passes each field a
 * `value` + `onChange`, so every field here is a dumb controlled input.
 *
 * The primitives call `useT()` only for their *intrinsic* chrome (the
 * "Expression" affordance, "Add row"/"Remove", "Invalid JSON" message) — the
 * semantic per-field *labels* are always passed in by the form so the i18n key
 * lives next to the field's meaning, not buried in a generic input.
 *
 * `=`-expression affordance: tinyflows resolves any config string starting with
 * `=` as an expression evaluated against the node's input (`crate::expr`).
 * {@link ExpressionField} surfaces that with a monospace input and a small
 * "Expression" badge + hint so an author knows a value like `=item.url` is live,
 * not a literal.
 */
import createDebug from 'debug';
import { useCallback, useId, useMemo, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import type { FlowConnection } from '../../../../services/api/flowsApi';
import type { UpstreamExpressionOption } from './upstreamOptions';

const log = createDebug('app:flows:nodeConfig:fields');

export const INPUT_CLASS =
  'w-full rounded-lg border border-line-strong bg-surface px-2.5 py-1.5 text-sm text-content ' +
  'placeholder-content-faint transition-colors focus:border-primary-500 focus:outline-none ' +
  'focus:ring-2 focus:ring-primary-500/20 disabled:opacity-50';
export const MONO_CLASS = 'font-mono text-[13px]';

/** Read a string field off a free-form config object, defaulting to `''`. */
export function configString(config: Record<string, unknown>, key: string): string {
  const value = config[key];
  return typeof value === 'string' ? value : '';
}

/** Read a `Record<string,string>` map off config (e.g. HTTP headers / transform set). */
export function configStringMap(
  config: Record<string, unknown>,
  key: string
): Record<string, string> {
  const value = config[key];
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
    out[k] = typeof v === 'string' ? v : JSON.stringify(v);
  }
  return out;
}

/** Label + optional hint wrapper shared by every field. */
export function Field({
  label,
  hint,
  htmlFor,
  children,
}: {
  label: string;
  hint?: string;
  htmlFor?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1">
      <label
        htmlFor={htmlFor}
        className="block text-[11px] font-semibold uppercase tracking-wide text-content-muted">
        {label}
      </label>
      {children}
      {hint && <p className="text-[11px] leading-snug text-content-faint">{hint}</p>}
    </div>
  );
}

export interface TextFieldProps {
  label: string;
  hint?: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  testId?: string;
}

export function TextField({ label, hint, value, onChange, placeholder, testId }: TextFieldProps) {
  const id = useId();
  return (
    <Field label={label} hint={hint} htmlFor={id}>
      <input
        id={id}
        type="text"
        className={INPUT_CLASS}
        value={value}
        placeholder={placeholder}
        data-testid={testId}
        onChange={e => onChange(e.target.value)}
      />
    </Field>
  );
}

export interface TextAreaFieldProps extends Omit<TextFieldProps, 'onChange'> {
  onChange: (value: string) => void;
  rows?: number;
  mono?: boolean;
}

export function TextAreaField({
  label,
  hint,
  value,
  onChange,
  placeholder,
  rows = 4,
  mono,
  testId,
}: TextAreaFieldProps) {
  const id = useId();
  return (
    <Field label={label} hint={hint} htmlFor={id}>
      <textarea
        id={id}
        rows={rows}
        className={`${INPUT_CLASS} resize-y ${mono ? MONO_CLASS : ''}`}
        value={value}
        placeholder={placeholder}
        data-testid={testId}
        onChange={e => onChange(e.target.value)}
      />
    </Field>
  );
}

export interface SelectOption {
  value: string;
  label: string;
}

export interface SelectFieldProps {
  label: string;
  hint?: string;
  value: string;
  onChange: (value: string) => void;
  options: SelectOption[];
  testId?: string;
}

export function SelectField({ label, hint, value, onChange, options, testId }: SelectFieldProps) {
  const id = useId();
  return (
    <Field label={label} hint={hint} htmlFor={id}>
      <select
        id={id}
        className={INPUT_CLASS}
        value={value}
        data-testid={testId}
        onChange={e => onChange(e.target.value)}>
        {options.map(opt => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>
    </Field>
  );
}

/**
 * Canonical model route hints (mirrors `AgentEditorPage`'s list, which in turn
 * mirrors the Rust `ModelSpec::Hint(...)` slugs). Selecting one routes the
 * agent node by capability tier; the workspace resolves the concrete model.
 */
export const AGENT_MODEL_HINTS = [
  'hint:reasoning',
  'hint:chat',
  'hint:agentic',
  'hint:burst',
  'hint:coding',
  'hint:summarization',
  'hint:vision',
] as const;

/** Sentinel select value for "type a raw model id" — never persisted. */
const CUSTOM_MODEL = '__custom__';

export interface ModelHintFieldProps {
  label: string;
  hint?: string;
  value: string;
  onChange: (value: string) => void;
  testId?: string;
}

/**
 * Model selector for the `agent` node: a dropdown of the workspace's model
 * route hints (`hint:chat`, `hint:coding`, …) with an "inherit" default and a
 * custom escape hatch for a raw BYOK model id. Writes `hint:<tier>` (or the raw
 * id, or `''` to inherit) onto `config.model`. Mirrors the agent-editor model
 * picker so hints stay consistent across the app.
 */
export function ModelHintField({ label, hint, value, onChange, testId }: ModelHintFieldProps) {
  const { t } = useT();
  const id = useId();
  // A value that's neither empty nor a known hint is a raw custom model id, so
  // the picker opens in custom mode showing it in the text box.
  const isKnown = value === '' || (AGENT_MODEL_HINTS as readonly string[]).includes(value);
  const [customMode, setCustomMode] = useState(value !== '' && !isKnown);

  const handleSelect = useCallback(
    (next: string) => {
      if (next === CUSTOM_MODEL) {
        setCustomMode(true);
        // Entering custom from a hint/inherit starts with an empty raw id.
        if (isKnown) onChange('');
        return;
      }
      setCustomMode(false);
      onChange(next);
    },
    [isKnown, onChange]
  );

  return (
    <Field label={label} hint={hint} htmlFor={id}>
      <div className="space-y-2">
        <select
          id={id}
          className={INPUT_CLASS}
          value={customMode ? CUSTOM_MODEL : value}
          data-testid={testId}
          onChange={e => handleSelect(e.target.value)}>
          <option value="">{t('flows.nodeConfig.agent.modelInherit')}</option>
          <optgroup label={t('flows.nodeConfig.agent.modelHints')}>
            {AGENT_MODEL_HINTS.map(h => (
              <option key={h} value={h}>
                {h}
              </option>
            ))}
          </optgroup>
          <option value={CUSTOM_MODEL}>{t('flows.nodeConfig.agent.modelCustom')}</option>
        </select>
        {customMode && (
          <input
            type="text"
            className={`${INPUT_CLASS} ${MONO_CLASS}`}
            value={value}
            placeholder={t('flows.nodeConfig.agent.modelCustomPlaceholder')}
            aria-label={t('flows.nodeConfig.agent.modelCustomPlaceholder')}
            data-testid={testId ? `${testId}-custom` : undefined}
            onChange={e => onChange(e.target.value)}
          />
        )}
      </div>
    </Field>
  );
}

/**
 * Compact "insert an upstream value" dropdown (Feature: `nodes` scope picker).
 * Always renders with the empty placeholder selected; picking an option calls
 * `onInsert(expression)` and snaps back to the placeholder, so it acts as a
 * one-shot menu button rather than a stateful select (same sentinel-select
 * pattern as `composioFields.tsx`). Renders nothing when there are no options.
 */
export function UpstreamInsertSelect({
  options,
  onInsert,
  testId,
  className,
}: {
  options: UpstreamExpressionOption[];
  onInsert: (expression: string) => void;
  testId?: string;
  className?: string;
}) {
  const { t } = useT();
  if (options.length === 0) return null;
  return (
    <select
      className={
        className ??
        'max-w-[45%] shrink-0 cursor-pointer border-l border-line-strong bg-surface-muted px-1.5 text-[11px] text-content-muted focus:outline-none'
      }
      value=""
      title={t('flows.nodeConfig.upstream.insertLabel', 'Insert a value from a previous step')}
      aria-label={t('flows.nodeConfig.upstream.insertLabel', 'Insert a value from a previous step')}
      data-testid={testId}
      onChange={e => {
        if (!e.target.value) return;
        log('UpstreamInsertSelect: insert %s', e.target.value);
        onInsert(e.target.value);
      }}>
      <option value="" disabled>
        {t('flows.nodeConfig.upstream.insert', 'Insert…')}
      </option>
      {options.map(opt => (
        <option key={opt.value} value={opt.value}>
          {opt.label}
        </option>
      ))}
    </select>
  );
}

export interface ExpressionFieldProps extends TextFieldProps {
  /**
   * `=nodes.…` expressions from upstream nodes; when non-empty a compact
   * insert dropdown renders beside the input (picking replaces the value).
   */
  upstreamOptions?: UpstreamExpressionOption[];
  /** Non-blocking warning (amber border + message), e.g. "Required — not wired". */
  warning?: string;
}

/**
 * A field whose value is commonly a tinyflows `=`-expression. Monospace input
 * with a leading "Expression" badge + hint so authors recognize `=item.foo`
 * as a live, input-bound value rather than a literal.
 */
export function ExpressionField({
  label,
  hint,
  value,
  onChange,
  placeholder,
  testId,
  upstreamOptions,
  warning,
}: ExpressionFieldProps) {
  const { t } = useT();
  const id = useId();
  const borderClass = warning
    ? 'border-amber-400 focus-within:border-amber-500 focus-within:ring-amber-500/20'
    : 'border-line-strong focus-within:border-primary-500 focus-within:ring-primary-500/20';
  return (
    <Field label={label} hint={hint ?? t('flows.nodeConfig.expressionHint')} htmlFor={id}>
      <div
        className={`flex items-stretch overflow-hidden rounded-lg border bg-surface focus-within:ring-2 ${borderClass}`}>
        <span
          className="flex select-none items-center border-r border-line-strong bg-surface-muted px-2 font-mono text-[11px] font-semibold text-content-muted"
          title={t('flows.nodeConfig.expressionBadge')}
          aria-hidden="true">
          =
        </span>
        <input
          id={id}
          type="text"
          className={`w-full bg-transparent px-2.5 py-1.5 ${MONO_CLASS} text-content placeholder-content-faint focus:outline-none`}
          value={value}
          placeholder={placeholder}
          data-testid={testId}
          onChange={e => onChange(e.target.value)}
        />
        {upstreamOptions && upstreamOptions.length > 0 && (
          <UpstreamInsertSelect
            options={upstreamOptions}
            onInsert={onChange}
            testId={testId ? `${testId}-upstream` : undefined}
          />
        )}
      </div>
      {warning && (
        <p className="text-[11px] font-medium text-amber-600 dark:text-amber-400" role="alert">
          {warning}
        </p>
      )}
    </Field>
  );
}

export interface KeyMapFieldProps {
  label: string;
  hint?: string;
  value: Record<string, string>;
  onChange: (value: Record<string, string>) => void;
  monoValues?: boolean;
  /** When non-empty, each row's value gets an upstream-expression insert menu. */
  upstreamOptions?: UpstreamExpressionOption[];
  testId?: string;
}

/**
 * Edits a flat `Record<string,string>` (HTTP headers, transform `set`) as a
 * list of key/value rows with add/remove. Rebuilds the whole object on every
 * keystroke so the parent stays the single controlled source of truth.
 */
export function KeyMapField({
  label,
  hint,
  value,
  onChange,
  monoValues,
  upstreamOptions,
  testId,
}: KeyMapFieldProps) {
  const { t } = useT();
  // Rows are kept as an ordered array locally so editing a key doesn't reorder
  // or drop an in-progress empty key. Seeded from `value`; the parent object is
  // rebuilt from rows on every change.
  const [rows, setRows] = useState<Array<[string, string]>>(() => Object.entries(value));

  const commit = useCallback(
    (next: Array<[string, string]>) => {
      setRows(next);
      const obj: Record<string, string> = {};
      for (const [k, v] of next) {
        if (k.trim() !== '') obj[k] = v;
      }
      log('KeyMapField commit: rows=%d keys=%d', next.length, Object.keys(obj).length);
      onChange(obj);
    },
    [onChange]
  );

  return (
    <Field label={label} hint={hint}>
      <div className="space-y-1.5" data-testid={testId}>
        {rows.map(([k, v], i) => (
          <div key={i} className="flex items-center gap-1.5">
            <input
              type="text"
              className={`${INPUT_CLASS} flex-1`}
              value={k}
              placeholder={t('flows.nodeConfig.keymapKeyPlaceholder')}
              onChange={e => {
                const next = rows.slice();
                next[i] = [e.target.value, v];
                commit(next);
              }}
            />
            <input
              type="text"
              className={`${INPUT_CLASS} flex-1 ${monoValues ? MONO_CLASS : ''}`}
              value={v}
              placeholder={t('flows.nodeConfig.keymapValuePlaceholder')}
              onChange={e => {
                const next = rows.slice();
                next[i] = [k, e.target.value];
                commit(next);
              }}
            />
            {upstreamOptions && upstreamOptions.length > 0 && (
              <UpstreamInsertSelect
                options={upstreamOptions}
                onInsert={expr => {
                  const next = rows.slice();
                  next[i] = [k, expr];
                  commit(next);
                }}
                className="w-20 shrink-0 cursor-pointer rounded-md border border-line-strong bg-surface-muted px-1 py-1 text-[11px] text-content-muted focus:outline-none"
              />
            )}
            <button
              type="button"
              className="shrink-0 rounded-md px-1.5 py-1 text-content-faint hover:bg-surface-hover hover:text-coral-600"
              aria-label={t('flows.nodeConfig.keymapRemove')}
              onClick={() => commit(rows.filter((_, idx) => idx !== i))}>
              ✕
            </button>
          </div>
        ))}
        <button
          type="button"
          className="rounded-md border border-dashed border-line-strong px-2 py-1 text-xs text-content-muted hover:bg-surface-hover"
          data-testid={testId ? `${testId}-add` : undefined}
          onClick={() => commit([...rows, ['', '']])}>
          + {t('flows.nodeConfig.keymapAdd')}
        </button>
      </div>
    </Field>
  );
}

export interface JsonFieldProps {
  label: string;
  hint?: string;
  value: unknown;
  onChange: (value: unknown) => void;
  rows?: number;
  testId?: string;
}

/**
 * Edits an arbitrary JSON value (HTTP body, tool_call args, or the raw-config
 * escape hatch) as pretty-printed text. Keeps a local text buffer so invalid
 * intermediate states are allowed while typing; only propagates `onChange` when
 * the buffer parses. Seeded once from `value` — the drawer body is keyed by
 * node id, so switching nodes remounts and re-seeds.
 */
export function JsonField({ label, hint, value, onChange, rows = 6, testId }: JsonFieldProps) {
  const { t } = useT();
  const id = useId();
  const initial = useMemo(() => {
    if (value === undefined || value === null) return '';
    try {
      return JSON.stringify(value, null, 2);
    } catch {
      return '';
    }
  }, [value]);
  const [text, setText] = useState(initial);
  const [error, setError] = useState(false);

  const handleChange = useCallback(
    (next: string) => {
      setText(next);
      if (next.trim() === '') {
        setError(false);
        onChange(null);
        return;
      }
      try {
        const parsed = JSON.parse(next);
        setError(false);
        log('JsonField parsed ok');
        onChange(parsed);
      } catch {
        setError(true);
        log('JsonField parse error — buffer held, not propagated');
      }
    },
    [onChange]
  );

  return (
    <Field label={label} hint={hint} htmlFor={id}>
      <textarea
        id={id}
        rows={rows}
        className={`${INPUT_CLASS} resize-y ${MONO_CLASS} ${
          error ? 'border-coral-400 focus:border-coral-500 focus:ring-coral-500/20' : ''
        }`}
        value={text}
        data-testid={testId}
        onChange={e => handleChange(e.target.value)}
      />
      {error && (
        <p className="text-[11px] font-medium text-coral-600 dark:text-coral-400" role="alert">
          {t('flows.nodeConfig.rawJsonInvalid')}
        </p>
      )}
    </Field>
  );
}

export interface CredentialPickerFieldProps {
  label?: string;
  value: string;
  onChange: (value: string) => void;
  connections: FlowConnection[];
  /** Restrict the offered connections by kind (e.g. only `http` for HTTP nodes). */
  kinds?: Array<FlowConnection['kind']>;
  testId?: string;
}

/**
 * Credential selector for `http_request` / `tool_call` nodes, fed by
 * `flows_list_connections` (secret-free). Writes the chosen `connection_ref`
 * onto config; the empty option clears it. Shows only `display`/`kind` — never
 * a secret. Renders a muted note when no credentials are connected.
 */
export function CredentialPickerField({
  label,
  value,
  onChange,
  connections,
  kinds,
  testId,
}: CredentialPickerFieldProps) {
  const { t } = useT();
  const id = useId();
  const options = useMemo(
    () => (kinds ? connections.filter(c => kinds.includes(c.kind)) : connections),
    [connections, kinds]
  );

  const resolvedLabel = label ?? t('flows.nodeConfig.credentialLabel');

  if (options.length === 0) {
    return (
      <Field label={resolvedLabel} hint={t('flows.nodeConfig.credentialHint')}>
        <p
          className="rounded-lg border border-dashed border-line-strong px-2.5 py-1.5 text-xs text-content-faint"
          data-testid={testId ? `${testId}-empty` : undefined}>
          {t('flows.nodeConfig.credentialEmpty')}
        </p>
      </Field>
    );
  }

  return (
    <Field label={resolvedLabel} hint={t('flows.nodeConfig.credentialHint')} htmlFor={id}>
      <select
        id={id}
        className={INPUT_CLASS}
        value={value}
        data-testid={testId}
        onChange={e => onChange(e.target.value)}>
        <option value="">{t('flows.nodeConfig.credentialNone')}</option>
        {options.map(conn => (
          <option key={conn.connection_ref} value={conn.connection_ref}>
            {conn.display} · {conn.kind}
          </option>
        ))}
      </select>
    </Field>
  );
}
