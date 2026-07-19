/**
 * AgentNodeInspector (Phase E) — the canvas-side controls for an `agent` node's
 * two harness-facing knobs, surfaced inside the node-config drawer's
 * {@link AgentForm}:
 *
 *  - `agent_ref` — which REGISTERED agent runs this node. Phase A routes an
 *    agent node whose `config.agent_ref` names a harness `AgentDefinition`
 *    through the FULL agent tool loop (that definition's ToolScope / sandbox /
 *    max-iterations govern the turn); leaving it blank runs the bare
 *    persona-shaped completion. The options come from the same
 *    `openhuman.agent_registry_list` RPC the Settings → Agents panel uses.
 *    `agent_ref` is trusted config (never model output), so this picker is the
 *    only way it's set from the UI.
 *  - `model` — a MANAGED capability tier (`reasoning-v1` ≈ Opus-class,
 *    `chat-v1` ≈ Sonnet-class, `agentic-v1`, `burst-v1`) the workspace resolves
 *    to a concrete model, with a free-form escape hatch for a raw BYOK model id.
 *    Matches the bare tier slugs Phase A's `OpenHumanAgentRunner` resolves and
 *    the Opus+Sonnet demo template (Phase C) hard-codes.
 *
 * Presentational + controlled: every edit calls `onChange` with a shallow-merge
 * config patch, exactly like the other node-config field groups. The agent list
 * is fetched once on mount; a fetch failure degrades to the inherit + custom
 * options only (the picker never blocks editing).
 */
import createDebug from 'debug';
import { useEffect, useId, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { agentRegistryApi, type AgentRegistryEntry } from '../../../services/api/agentRegistryApi';
import { configString, SelectField } from './nodeConfig/nodeConfigFields';

const log = createDebug('app:flows:canvas:agentInspector');

/**
 * The managed capability tiers offered for an agent node's `model`. Mirrors the
 * Rust `MODEL_*_V1` constants (`src/openhuman/config/schema/types.rs`) and the
 * slugs `OpenHumanAgentRunner`/`resolve_model_for_hint` accept as bare tier
 * names — so the value written here runs unchanged in the flow engine.
 */
export const AGENT_MANAGED_TIERS = ['reasoning-v1', 'chat-v1', 'agentic-v1', 'burst-v1'] as const;

/** Sentinel select value for "type a raw model id" — never persisted. */
const CUSTOM_MODEL = '__custom__';

export interface AgentNodeInspectorProps {
  /** The agent node's controlled config object. */
  config: Record<string, unknown>;
  /** Shallow-merge patch into the node's config. */
  onChange: (patch: Record<string, unknown>) => void;
}

/**
 * The `agent_ref` picker: an "inherit" default plus every registered agent. A
 * currently-set ref that isn't in the fetched list (still loading, or points at
 * a now-removed agent) is preserved as its own option so the value never
 * silently drops.
 */
function AgentRefField({ config, onChange }: AgentNodeInspectorProps) {
  const { t } = useT();
  const [agents, setAgents] = useState<AgentRegistryEntry[]>([]);
  const value = configString(config, 'agent_ref');

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await agentRegistryApi.list(false);
        if (cancelled) return;
        log('AgentRefField: loaded %d agent(s)', list.length);
        setAgents(list);
      } catch (err) {
        // Non-fatal: keep just the inherit + preserved-value options so the
        // drawer still edits. See module doc.
        log('AgentRefField: agent_registry_list failed — inherit only: %o', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const options = [
    { value: '', label: t('flows.nodeConfig.agent.agentRefInherit') },
    ...agents.map(a => ({ value: a.id, label: a.name })),
  ];
  // Preserve an out-of-list ref so it stays selected and visible.
  if (value && !options.some(o => o.value === value)) {
    options.push({ value, label: value });
  }

  return (
    <SelectField
      label={t('flows.nodeConfig.agent.agentRefLabel')}
      hint={t('flows.nodeConfig.agent.agentRefHint')}
      value={value}
      onChange={v => onChange({ agent_ref: v })}
      options={options}
      testId="node-config-agent-ref"
    />
  );
}

/**
 * The managed-tier `model` picker with a custom escape hatch. Writes a bare tier
 * slug (`reasoning-v1`…), a raw model id, or `''` to inherit onto `config.model`.
 */
function ManagedModelField({ config, onChange }: AgentNodeInspectorProps) {
  const { t } = useT();
  const id = useId();
  const value = configString(config, 'model');
  const isKnown = value === '' || (AGENT_MANAGED_TIERS as readonly string[]).includes(value);
  const [customMode, setCustomMode] = useState(value !== '' && !isKnown);

  const handleSelect = (next: string) => {
    if (next === CUSTOM_MODEL) {
      setCustomMode(true);
      // Entering custom from a tier/inherit starts with an empty raw id.
      if (isKnown) onChange({ model: '' });
      return;
    }
    setCustomMode(false);
    onChange({ model: next });
  };

  return (
    <div className="space-y-1.5">
      <label
        htmlFor={id}
        className="block text-[11px] font-medium uppercase tracking-wide text-content-faint">
        {t('flows.nodeConfig.agent.modelLabel')}
      </label>
      <p className="text-[11px] text-content-muted">{t('flows.nodeConfig.agent.modelHint')}</p>
      <select
        id={id}
        className="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 text-sm text-content focus:border-primary-400 focus:outline-none"
        value={customMode ? CUSTOM_MODEL : value}
        data-testid="node-config-agent-model"
        onChange={e => handleSelect(e.target.value)}>
        <option value="">{t('flows.nodeConfig.agent.modelInherit')}</option>
        <optgroup label={t('flows.nodeConfig.agent.modelManagedTiers')}>
          {AGENT_MANAGED_TIERS.map(tier => (
            <option key={tier} value={tier}>
              {tier}
            </option>
          ))}
        </optgroup>
        <option value={CUSTOM_MODEL}>{t('flows.nodeConfig.agent.modelCustom')}</option>
      </select>
      {customMode && (
        <input
          type="text"
          className="w-full rounded-lg border border-line bg-surface px-2.5 py-1.5 font-mono text-sm text-content focus:border-primary-400 focus:outline-none"
          value={value}
          placeholder={t('flows.nodeConfig.agent.modelCustomPlaceholder')}
          aria-label={t('flows.nodeConfig.agent.modelCustomPlaceholder')}
          data-testid="node-config-agent-model-custom"
          onChange={e => onChange({ model: e.target.value })}
        />
      )}
    </div>
  );
}

/**
 * Agent-node inspector: the `agent_ref` picker over `model` tier picker. Sits
 * inside {@link AgentForm} below the prompt so the two harness knobs read as one
 * group.
 */
export default function AgentNodeInspector({ config, onChange }: AgentNodeInspectorProps) {
  return (
    <div className="space-y-3" data-testid="agent-node-inspector">
      <AgentRefField config={config} onChange={onChange} />
      <ManagedModelField config={config} onChange={onChange} />
    </div>
  );
}
