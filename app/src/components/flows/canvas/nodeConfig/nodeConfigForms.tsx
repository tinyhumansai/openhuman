/**
 * Per-kind config forms for the node-config drawer (issue B5b / Phase 3b).
 * Each form renders a small set of typed fields for a high-traffic `NodeKind`,
 * reading from the controlled `config` object and merging edits back via
 * `onChange` (a shallow-merge patch). Kinds without a dedicated form fall back
 * to the drawer's raw-JSON editor.
 *
 * Field keys mirror what `vendor/tinyflows` actually reads at runtime:
 *  - `http_request` → `method` / `url` / `connection_ref` / `headers` / `body`
 *  - `agent`        → `prompt` / `model` / `connection_ref`
 *  - `tool_call`    → `slug` / `args` / `connection_ref`
 *  - `code`         → `language` (`javascript`|`python`) / `source`
 *  - `condition`    → `field` (truthiness of a key on the input item)
 *  - `switch`       → `expression` (=-expr, precedence) / `field` (fallback)
 *  - `transform`    → `set` (key → =-expression map)
 *  - `trigger`      → `trigger_kind` + kind-specific (`schedule`, `toolkit`/`trigger_slug`)
 */
import createDebug from 'debug';
import { useEffect, useState } from 'react';

import { describeSchedule, scheduleCronExpr } from '../../../../lib/flows/cron';
import type { NodeKind } from '../../../../lib/flows/types';
import { useT } from '../../../../lib/i18n/I18nContext';
import type { FlowConnection } from '../../../../services/api/flowsApi';
import AgentNodeInspector from '../AgentNodeInspector';
import {
  ComposioActionField,
  type ComposioActionSchema,
  ComposioToolkitField,
  ComposioTriggerField,
  fetchActionSchema,
} from './composioFields';
import { NATIVE_TOOL_PREFIX, NativeToolField } from './nativeToolFields';
import {
  configString,
  configStringMap,
  CredentialPickerField,
  ExpressionField,
  JsonField,
  KeyMapField,
  SelectField,
  TextAreaField,
  TextField,
  UpstreamInsertSelect,
} from './nodeConfigFields';
import { ScheduleField } from './ScheduleField';
import type { UpstreamExpressionOption } from './upstreamOptions';

const log = createDebug('app:flows:nodeConfig:forms');

/** Derive the toolkit slug from a `composio:<toolkit>:<conn>` connection ref. */
function toolkitFromConnectionRef(ref: string): string {
  const parts = ref.split(':');
  return parts[0] === 'composio' && parts.length >= 2 ? parts[1] : '';
}

export interface NodeConfigFormProps {
  config: Record<string, unknown>;
  /** Shallow-merge patch into the node's config (undefined values are still set). */
  onChange: (patch: Record<string, unknown>) => void;
  connections: FlowConnection[];
  /**
   * `=nodes.…` expressions referencing this node's upstream ancestors, for the
   * insert pickers on expression-bearing fields. Optional — absent (or empty)
   * simply hides the pickers.
   */
  upstreamOptions?: UpstreamExpressionOption[];
}

export type NodeConfigForm = (props: NodeConfigFormProps) => React.ReactElement;

// ── trigger ────────────────────────────────────────────────────────────────

const TRIGGER_KINDS = ['manual', 'schedule', 'webhook', 'app_event'] as const;

function TriggerForm({ config, onChange, connections }: NodeConfigFormProps) {
  const { t } = useT();
  const kind = configString(config, 'trigger_kind') || 'manual';
  const toolkit = configString(config, 'toolkit');
  return (
    <div className="space-y-3">
      <SelectField
        label={t('flows.nodeConfig.trigger.kindLabel')}
        value={kind}
        onChange={v => onChange({ trigger_kind: v })}
        testId="node-config-trigger-kind"
        options={TRIGGER_KINDS.map(k => ({
          value: k,
          label: t(`flows.nodeConfig.trigger.kind_${k}`),
        }))}
      />
      {kind === 'schedule' &&
        (() => {
          const rawSchedule = config.schedule;
          const cronExpr = scheduleCronExpr(rawSchedule);
          // A cron-shaped schedule (bare string or `{kind:"cron", expr}`) — or
          // nothing set yet — is what the visual/advanced builder understands;
          // let it render (and start empty, as before, when unset).
          if (cronExpr !== null || rawSchedule == null) {
            return (
              <ScheduleField
                value={cronExpr ?? ''}
                onChange={v => onChange({ schedule: v })}
                testId="node-config-trigger-schedule"
              />
            );
          }
          // `{kind:"every", every_ms}` / `{kind:"at", at}` — the cron builder
          // can't model these. Show a read-only summary instead of handing it
          // to `ScheduleField`, whose mount effect would otherwise seed (and
          // silently overwrite) a default cron string over the real schedule.
          return (
            <div
              className="rounded-lg border border-primary-200 bg-primary-50/60 px-2.5 py-1.5 text-xs font-medium text-primary-700 dark:border-primary-500/30 dark:bg-primary-500/10 dark:text-primary-300"
              data-testid="node-config-trigger-schedule-readonly">
              {describeSchedule(rawSchedule)}
            </div>
          );
        })()}
      {kind === 'app_event' && (
        <>
          <ComposioToolkitField
            label={t('flows.nodeConfig.trigger.toolkitLabel')}
            value={toolkit}
            // Changing the app clears a now-mismatched trigger slug.
            onChange={v => onChange({ toolkit: v, trigger_slug: '' })}
            connections={connections}
            testId="node-config-trigger-toolkit"
          />
          <ComposioTriggerField
            label={t('flows.nodeConfig.trigger.triggerSlugLabel')}
            value={configString(config, 'trigger_slug')}
            onChange={v => onChange({ trigger_slug: v })}
            toolkit={toolkit}
            testId="node-config-trigger-slug"
          />
        </>
      )}
      {kind === 'webhook' && (
        <p className="rounded-lg border border-dashed border-amber-300 bg-amber-50 px-2.5 py-1.5 text-[11px] text-amber-700 dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-300">
          {t('flows.nodeConfig.trigger.webhookHint')}
        </p>
      )}
    </div>
  );
}

// ── http_request ─────────────────────────────────────────────────────────────

const HTTP_METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'];

function HttpRequestForm({ config, onChange, connections, upstreamOptions }: NodeConfigFormProps) {
  const { t } = useT();
  return (
    <div className="space-y-3">
      <SelectField
        label={t('flows.nodeConfig.http.methodLabel')}
        value={configString(config, 'method') || 'GET'}
        onChange={v => onChange({ method: v })}
        testId="node-config-http-method"
        options={HTTP_METHODS.map(m => ({ value: m, label: m }))}
      />
      <ExpressionField
        label={t('flows.nodeConfig.http.urlLabel')}
        value={configString(config, 'url')}
        onChange={v => onChange({ url: v })}
        placeholder="https://api.example.com/v1/resource"
        upstreamOptions={upstreamOptions}
        testId="node-config-http-url"
      />
      <CredentialPickerField
        value={configString(config, 'connection_ref')}
        onChange={v => onChange({ connection_ref: v })}
        connections={connections}
        kinds={['http']}
        testId="node-config-http-credential"
      />
      <KeyMapField
        label={t('flows.nodeConfig.http.headersLabel')}
        value={configStringMap(config, 'headers')}
        onChange={v => onChange({ headers: v })}
        monoValues
        testId="node-config-http-headers"
      />
      <JsonField
        label={t('flows.nodeConfig.http.bodyLabel')}
        value={config.body ?? null}
        onChange={v => onChange({ body: v })}
        testId="node-config-http-body"
      />
    </div>
  );
}

// ── agent ────────────────────────────────────────────────────────────────────

function AgentForm({ config, onChange, connections, upstreamOptions }: NodeConfigFormProps) {
  const { t } = useT();
  const prompt = configString(config, 'prompt');
  return (
    <div className="space-y-3">
      <TextAreaField
        label={t('flows.nodeConfig.agent.promptLabel')}
        value={prompt}
        onChange={v => onChange({ prompt: v })}
        placeholder={t('flows.nodeConfig.agent.promptPlaceholder')}
        rows={5}
        testId="node-config-agent-prompt"
      />
      {upstreamOptions && upstreamOptions.length > 0 && (
        // Appends the picked `=nodes.…` expression to the prompt (a prompt is
        // prose, so inserting must not clobber what's already written).
        <div className="-mt-2 flex justify-end">
          <UpstreamInsertSelect
            options={upstreamOptions}
            onInsert={expr => onChange({ prompt: prompt ? `${prompt} ${expr}` : expr })}
            testId="node-config-agent-prompt-upstream"
            className="cursor-pointer rounded-md border border-line-strong bg-surface-muted px-1.5 py-1 text-[11px] text-content-muted focus:outline-none"
          />
        </div>
      )}
      {/* Harness knobs: which registered agent runs this node (Phase A routes an
          `agent_ref` node through the full tool loop) + its managed model tier.
          Both write onto the node config via the same shallow-merge patch. */}
      <AgentNodeInspector config={config} onChange={onChange} />
      <CredentialPickerField
        value={configString(config, 'connection_ref')}
        onChange={v => onChange({ connection_ref: v })}
        connections={connections}
        testId="node-config-agent-credential"
      />
    </div>
  );
}

// ── tool_call ─────────────────────────────────────────────────────────────────

/** Read `config.args` as a plain object (Composio args are a JSON object). */
function configArgs(config: Record<string, unknown>): Record<string, unknown> {
  const args = config.args;
  return args && typeof args === 'object' && !Array.isArray(args)
    ? (args as Record<string, unknown>)
    : {};
}

function ToolCallForm({ config, onChange, connections, upstreamOptions }: NodeConfigFormProps) {
  const { t } = useT();
  const slug = configString(config, 'slug');
  // Two flavours of tool_call: a native OpenHuman "Tool" (provider=openhuman /
  // slug `oh:...`) vs a Composio "App action". The palette seeds `provider`.
  const isNative =
    configString(config, 'provider') === 'openhuman' || slug.startsWith(NATIVE_TOOL_PREFIX);

  // The connected account is chosen first; its toolkit scopes the action list.
  const connectionRef = configString(config, 'connection_ref');
  const toolkit = toolkitFromConnectionRef(connectionRef);

  // Required-arg preflight rows (Composio only): once an action is selected,
  // fetch its JSON-schema `parameters` so each required arg gets its own
  // labeled ExpressionField row. `null` (fetch failed / unknown action /
  // native tool) degrades gracefully to just the raw JsonField below. The
  // fetched schema is tagged with its toolkit/slug key so switching action
  // discards a stale schema without a synchronous setState in the effect.
  const schemaKey = `${toolkit} ${slug}`;
  const [fetchedSchema, setFetchedSchema] = useState<{
    key: string;
    schema: ComposioActionSchema | null;
  } | null>(null);
  const actionSchema = fetchedSchema?.key === schemaKey ? fetchedSchema.schema : null;
  // Remount key for the raw args JsonField: it holds a local text buffer that
  // is seeded once, so row edits bump this to re-seed it with the merged args.
  const [argsSeed, setArgsSeed] = useState(0);
  useEffect(() => {
    if (isNative || !toolkit || !slug) return;
    const key = `${toolkit} ${slug}`;
    let cancelled = false;
    void (async () => {
      try {
        const schema = await fetchActionSchema(toolkit, slug);
        if (!cancelled) {
          log(
            'ToolCallForm: schema %s/%s → required=%d optional=%d',
            toolkit,
            slug,
            schema?.required.length ?? -1,
            schema?.optional.length ?? -1
          );
          setFetchedSchema({ key, schema });
        }
      } catch {
        // Catalog fetch failed — keep `null` and fall back to the raw editor.
        log('ToolCallForm: schema fetch failed for %s/%s — raw args only', toolkit, slug);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isNative, toolkit, slug]);

  if (isNative) {
    return (
      <div className="space-y-3">
        <NativeToolField
          label={t('flows.nodeConfig.native.toolLabel')}
          hint={t('flows.nodeConfig.native.toolHint')}
          value={slug}
          onChange={v => onChange({ slug: v })}
          testId="node-config-tool-slug"
        />
        <JsonField
          label={t('flows.nodeConfig.tool.argsLabel')}
          value={config.args ?? null}
          onChange={v => onChange({ args: v })}
          testId="node-config-tool-args"
        />
      </div>
    );
  }

  const args = configArgs(config);
  const requiredArgs = actionSchema?.required ?? [];
  const setArg = (name: string, value: string) => {
    const next = { ...args };
    if (value === '') {
      delete next[name];
    } else {
      next[name] = value;
    }
    log('ToolCallForm: setArg %s (empty=%s)', name, value === '');
    onChange({ args: next });
    // Re-seed the raw editor so it reflects the row edit.
    setArgsSeed(s => s + 1);
  };

  return (
    <div className="space-y-3">
      <CredentialPickerField
        value={connectionRef}
        // Changing the account can change the toolkit → clear a stale action.
        onChange={v => onChange({ connection_ref: v, slug: '' })}
        connections={connections}
        kinds={['composio']}
        testId="node-config-tool-credential"
      />
      <ComposioActionField
        label={t('flows.nodeConfig.tool.slugLabel')}
        value={slug}
        onChange={v => onChange({ slug: v })}
        toolkit={toolkit}
        testId="node-config-tool-slug"
      />
      {requiredArgs.map(name => {
        const raw = args[name];
        const value = typeof raw === 'string' ? raw : raw == null ? '' : JSON.stringify(raw);
        const missing = value.trim() === '';
        return (
          <ExpressionField
            key={name}
            label={`${name} · ${t('flows.nodeConfig.tool.requiredMark', 'required')}`}
            hint=""
            value={value}
            onChange={v => setArg(name, v)}
            upstreamOptions={upstreamOptions}
            warning={
              missing
                ? t('flows.nodeConfig.tool.requiredMissing', 'Required — not wired')
                : undefined
            }
            testId={`node-config-tool-arg-${name}`}
          />
        );
      })}
      <JsonField
        key={argsSeed}
        label={
          requiredArgs.length > 0
            ? t('flows.nodeConfig.tool.argsAdvancedLabel', 'All args (advanced)')
            : t('flows.nodeConfig.tool.argsLabel')
        }
        value={config.args ?? null}
        onChange={v => onChange({ args: v })}
        testId="node-config-tool-args"
      />
    </div>
  );
}

// ── condition ─────────────────────────────────────────────────────────────────

function ConditionForm({ config, onChange }: NodeConfigFormProps) {
  const { t } = useT();
  return (
    <div className="space-y-3">
      <TextField
        label={t('flows.nodeConfig.condition.fieldLabel')}
        hint={t('flows.nodeConfig.condition.fieldHint')}
        value={configString(config, 'field')}
        onChange={v => onChange({ field: v })}
        placeholder="status"
        testId="node-config-condition-field"
      />
    </div>
  );
}

// ── switch ────────────────────────────────────────────────────────────────────

function SwitchForm({ config, onChange, upstreamOptions }: NodeConfigFormProps) {
  const { t } = useT();
  return (
    <div className="space-y-3">
      <ExpressionField
        label={t('flows.nodeConfig.switch.expressionLabel')}
        hint={t('flows.nodeConfig.switch.hint')}
        value={configString(config, 'expression')}
        onChange={v => onChange({ expression: v })}
        placeholder="item.type"
        upstreamOptions={upstreamOptions}
        testId="node-config-switch-expression"
      />
      <TextField
        label={t('flows.nodeConfig.switch.fieldLabel')}
        value={configString(config, 'field')}
        onChange={v => onChange({ field: v })}
        placeholder="type"
        testId="node-config-switch-field"
      />
    </div>
  );
}

// ── transform ─────────────────────────────────────────────────────────────────

function TransformForm({ config, onChange, upstreamOptions }: NodeConfigFormProps) {
  const { t } = useT();
  return (
    <div className="space-y-3">
      <KeyMapField
        label={t('flows.nodeConfig.transform.setLabel')}
        hint={t('flows.nodeConfig.transform.setHint')}
        value={configStringMap(config, 'set')}
        onChange={v => onChange({ set: v })}
        monoValues
        upstreamOptions={upstreamOptions}
        testId="node-config-transform-set"
      />
    </div>
  );
}

// ── code ──────────────────────────────────────────────────────────────────────

function CodeForm({ config, onChange }: NodeConfigFormProps) {
  const { t } = useT();
  const language = configString(config, 'language') || 'javascript';
  return (
    <div className="space-y-3">
      <SelectField
        label={t('flows.nodeConfig.code.languageLabel')}
        value={language}
        onChange={v => onChange({ language: v })}
        testId="node-config-code-language"
        options={[
          { value: 'javascript', label: t('flows.nodeConfig.code.language_javascript') },
          { value: 'python', label: t('flows.nodeConfig.code.language_python') },
        ]}
      />
      <TextAreaField
        label={t('flows.nodeConfig.code.sourceLabel')}
        value={configString(config, 'source')}
        onChange={v => onChange({ source: v })}
        placeholder="return items;"
        rows={8}
        mono
        testId="node-config-code-source"
      />
    </div>
  );
}

/**
 * Registry of the kinds that get a dedicated form. Any `NodeKind` absent here
 * (merge, split_out, output_parser, sub_workflow) falls through to the drawer's
 * raw-JSON escape hatch.
 */
export const NODE_CONFIG_FORMS: Partial<Record<NodeKind, NodeConfigForm>> = {
  trigger: TriggerForm,
  http_request: HttpRequestForm,
  agent: AgentForm,
  tool_call: ToolCallForm,
  condition: ConditionForm,
  switch: SwitchForm,
  transform: TransformForm,
  code: CodeForm,
};
