/**
 * Behavior tests for representative per-kind node-config forms (Phase 3b):
 * http_request (typed fields + kind-filtered credential picker), transform
 * (key→value map), and trigger (conditional fields by trigger_kind). Forms are
 * pulled from the {@link NODE_CONFIG_FORMS} registry and driven through their
 * public `onChange` patch contract. `useT()` falls back to the bundled English
 * map with no provider mounted (same as the sibling canvas tests).
 */
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AgentRegistryEntry } from '../../../../../services/api/agentRegistryApi';
import type { FlowConnection } from '../../../../../services/api/flowsApi';
import { NODE_CONFIG_FORMS, type NodeConfigFormProps } from '../nodeConfigForms';

// AgentForm now renders AgentNodeInspector, which fetches the agent registry on
// mount. Stub it so the form renders offline with a deterministic agent list.
const AGENTS: AgentRegistryEntry[] = [
  { id: 'researcher', name: 'Researcher', description: '', source: 'default', enabled: true },
  { id: 'drafter', name: 'Drafter', description: '', source: 'custom', enabled: true },
];
const agentListMock = vi.hoisted(() => vi.fn());
vi.mock('../../../../../services/api/agentRegistryApi', () => ({
  agentRegistryApi: { list: agentListMock },
}));
agentListMock.mockResolvedValue(AGENTS);

function renderForm(kind: keyof typeof NODE_CONFIG_FORMS, props: Partial<NodeConfigFormProps>) {
  const Form = NODE_CONFIG_FORMS[kind]!;
  const onChange = vi.fn();
  render(
    <Form config={props.config ?? {}} onChange={onChange} connections={props.connections ?? []} />
  );
  return { onChange };
}

const HTTP_CRED: FlowConnection = {
  connection_ref: 'http_cred:stripe',
  kind: 'http',
  display: 'Stripe API',
  scheme: 'bearer',
};
const COMPOSIO_CRED: FlowConnection = {
  connection_ref: 'composio:github:conn_1',
  kind: 'composio',
  display: 'GitHub',
  toolkit: 'github',
};

describe('HttpRequestForm', () => {
  it('emits a method patch when the method changes', () => {
    const { onChange } = renderForm('http_request', {});
    fireEvent.change(screen.getByTestId('node-config-http-method'), { target: { value: 'POST' } });
    expect(onChange).toHaveBeenLastCalledWith({ method: 'POST' });
  });

  it('emits a url patch as the URL is typed', () => {
    const { onChange } = renderForm('http_request', {});
    fireEvent.change(screen.getByTestId('node-config-http-url'), {
      target: { value: '=item.url' },
    });
    expect(onChange).toHaveBeenLastCalledWith({ url: '=item.url' });
  });

  it('offers only http-kind credentials in the picker and emits connection_ref on select', () => {
    const { onChange } = renderForm('http_request', { connections: [HTTP_CRED, COMPOSIO_CRED] });
    const select = screen.getByTestId('node-config-http-credential');
    // None + the single http credential — the composio one is filtered out.
    const options = within(select).getAllByRole('option');
    expect(options).toHaveLength(2);
    expect(screen.queryByText(/GitHub/)).not.toBeInTheDocument();

    fireEvent.change(select, { target: { value: 'http_cred:stripe' } });
    expect(onChange).toHaveBeenLastCalledWith({ connection_ref: 'http_cred:stripe' });
  });
});

describe('TransformForm', () => {
  it('builds a set map from added key/value rows', () => {
    const { onChange } = renderForm('transform', {});
    // Add a row, then fill key + value.
    fireEvent.click(screen.getByTestId('node-config-transform-set-add'));
    const container = screen.getByTestId('node-config-transform-set');
    const inputs = within(container).getAllByRole('textbox');
    expect(inputs).toHaveLength(2);
    fireEvent.change(inputs[0], { target: { value: 'greeting' } });
    fireEvent.change(inputs[1], { target: { value: '=item.name' } });
    expect(onChange).toHaveBeenLastCalledWith({ set: { greeting: '=item.name' } });
  });
});

describe('AgentForm', () => {
  it('offers managed tiers and patches config.model with the chosen tier', () => {
    const { onChange } = renderForm('agent', {});
    fireEvent.change(screen.getByTestId('node-config-agent-model'), {
      target: { value: 'reasoning-v1' },
    });
    expect(onChange).toHaveBeenLastCalledWith({ model: 'reasoning-v1' });
  });

  it('reveals a custom model input when Custom is selected', () => {
    const { onChange } = renderForm('agent', {});
    // No custom text box until Custom is picked.
    expect(screen.queryByTestId('node-config-agent-model-custom')).not.toBeInTheDocument();
    fireEvent.change(screen.getByTestId('node-config-agent-model'), {
      target: { value: '__custom__' },
    });
    const custom = screen.getByTestId('node-config-agent-model-custom');
    fireEvent.change(custom, { target: { value: 'gpt-4o-mini' } });
    expect(onChange).toHaveBeenLastCalledWith({ model: 'gpt-4o-mini' });
  });

  it('opens in custom mode when config.model is a raw model id', () => {
    renderForm('agent', { config: { model: 'claude-sonnet-5' } });
    expect(screen.getByTestId('node-config-agent-model-custom')).toHaveValue('claude-sonnet-5');
  });

  it('lists registered agents and patches config.agent_ref on select', async () => {
    const { onChange } = renderForm('agent', {});
    // The agent list loads from the (mocked) registry after mount.
    await screen.findByRole('option', { name: 'Researcher' });
    fireEvent.change(screen.getByTestId('node-config-agent-ref'), { target: { value: 'drafter' } });
    expect(onChange).toHaveBeenLastCalledWith({ agent_ref: 'drafter' });
  });

  it('preserves an out-of-list agent_ref as a selectable option', async () => {
    renderForm('agent', { config: { agent_ref: 'ghost-agent' } });
    await screen.findByRole('option', { name: 'Researcher' });
    // A ref that isn't in the fetched list is kept so the value never drops.
    expect(screen.getByTestId('node-config-agent-ref')).toHaveValue('ghost-agent');
  });
});

describe('TriggerForm', () => {
  it('reveals the cron schedule field only for the schedule kind and patches it', () => {
    const { onChange } = renderForm('trigger', {});
    // Manual by default — no schedule field.
    expect(screen.queryByTestId('node-config-trigger-schedule')).not.toBeInTheDocument();

    fireEvent.change(screen.getByTestId('node-config-trigger-kind'), {
      target: { value: 'schedule' },
    });
    expect(onChange).toHaveBeenLastCalledWith({ trigger_kind: 'schedule' });
  });

  it('shows the friendly schedule builder (with summary) for a schedule trigger', () => {
    renderForm('trigger', { config: { trigger_kind: 'schedule', schedule: '*/5 * * * 3' } });
    expect(screen.getByTestId('node-config-trigger-schedule')).toBeInTheDocument();
    // Compiled plain-English summary instead of a raw cron box.
    expect(screen.getByTestId('node-config-trigger-schedule-summary')).toHaveTextContent(
      'Every 5 minutes on Wed'
    );
  });

  it('shows a read-only summary (not the cron builder) for a tagged `{kind:"every"}` schedule', () => {
    // Regression: the engine can store `config.schedule` as `{kind:"every",
    // every_ms}`, which the cron builder can't edit. It must render a correct
    // read-only summary instead of silently resetting the schedule to a
    // default cron string via ScheduleField's empty-mount effect.
    const { onChange } = renderForm('trigger', {
      config: { trigger_kind: 'schedule', schedule: { kind: 'every', every_ms: 86_400_000 } },
    });
    expect(screen.queryByTestId('node-config-trigger-schedule')).not.toBeInTheDocument();
    expect(screen.getByTestId('node-config-trigger-schedule-readonly')).toHaveTextContent('24h');
    expect(onChange).not.toHaveBeenCalled();
  });

  it('still hands a tagged `{kind:"cron"}` schedule to the editable builder', () => {
    renderForm('trigger', {
      config: { trigger_kind: 'schedule', schedule: { kind: 'cron', expr: '30 9 * * *' } },
    });
    expect(screen.getByTestId('node-config-trigger-schedule')).toBeInTheDocument();
    expect(screen.getByTestId('node-config-trigger-schedule-summary')).toHaveTextContent(
      'Every day at 09:30'
    );
  });
});
