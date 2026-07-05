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
import type { NodeKind } from '../../../../lib/flows/types';
import { useT } from '../../../../lib/i18n/I18nContext';
import type { FlowConnection } from '../../../../services/api/flowsApi';
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
} from './nodeConfigFields';

export interface NodeConfigFormProps {
  config: Record<string, unknown>;
  /** Shallow-merge patch into the node's config (undefined values are still set). */
  onChange: (patch: Record<string, unknown>) => void;
  connections: FlowConnection[];
}

export type NodeConfigForm = (props: NodeConfigFormProps) => React.ReactElement;

// ── trigger ────────────────────────────────────────────────────────────────

const TRIGGER_KINDS = ['manual', 'schedule', 'webhook', 'app_event'] as const;

function TriggerForm({ config, onChange }: NodeConfigFormProps) {
  const { t } = useT();
  const kind = configString(config, 'trigger_kind') || 'manual';
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
      {kind === 'schedule' && (
        <TextField
          label={t('flows.nodeConfig.trigger.scheduleLabel')}
          hint={t('flows.nodeConfig.trigger.scheduleHint')}
          value={configString(config, 'schedule')}
          onChange={v => onChange({ schedule: v })}
          placeholder="0 9 * * 1"
          testId="node-config-trigger-schedule"
        />
      )}
      {kind === 'app_event' && (
        <>
          <TextField
            label={t('flows.nodeConfig.trigger.toolkitLabel')}
            value={configString(config, 'toolkit')}
            onChange={v => onChange({ toolkit: v })}
            placeholder="github"
          />
          <TextField
            label={t('flows.nodeConfig.trigger.triggerSlugLabel')}
            value={configString(config, 'trigger_slug')}
            onChange={v => onChange({ trigger_slug: v })}
            placeholder="GITHUB_STAR_ADDED"
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

function HttpRequestForm({ config, onChange, connections }: NodeConfigFormProps) {
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

function AgentForm({ config, onChange, connections }: NodeConfigFormProps) {
  const { t } = useT();
  return (
    <div className="space-y-3">
      <TextAreaField
        label={t('flows.nodeConfig.agent.promptLabel')}
        value={configString(config, 'prompt')}
        onChange={v => onChange({ prompt: v })}
        placeholder={t('flows.nodeConfig.agent.promptPlaceholder')}
        rows={5}
        testId="node-config-agent-prompt"
      />
      <TextField
        label={t('flows.nodeConfig.agent.modelLabel')}
        value={configString(config, 'model')}
        onChange={v => onChange({ model: v })}
        placeholder="gpt-4o-mini"
        testId="node-config-agent-model"
      />
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

function ToolCallForm({ config, onChange, connections }: NodeConfigFormProps) {
  const { t } = useT();
  return (
    <div className="space-y-3">
      <TextField
        label={t('flows.nodeConfig.tool.slugLabel')}
        value={configString(config, 'slug')}
        onChange={v => onChange({ slug: v })}
        placeholder="GITHUB_CREATE_ISSUE"
        testId="node-config-tool-slug"
      />
      <CredentialPickerField
        value={configString(config, 'connection_ref')}
        onChange={v => onChange({ connection_ref: v })}
        connections={connections}
        testId="node-config-tool-credential"
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

function SwitchForm({ config, onChange }: NodeConfigFormProps) {
  const { t } = useT();
  return (
    <div className="space-y-3">
      <ExpressionField
        label={t('flows.nodeConfig.switch.expressionLabel')}
        hint={t('flows.nodeConfig.switch.hint')}
        value={configString(config, 'expression')}
        onChange={v => onChange({ expression: v })}
        placeholder="item.type"
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

function TransformForm({ config, onChange }: NodeConfigFormProps) {
  const { t } = useT();
  return (
    <div className="space-y-3">
      <KeyMapField
        label={t('flows.nodeConfig.transform.setLabel')}
        hint={t('flows.nodeConfig.transform.setHint')}
        value={configStringMap(config, 'set')}
        onChange={v => onChange({ set: v })}
        monoValues
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
