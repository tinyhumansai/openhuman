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

import type { FlowConnection } from '../../../../../services/api/flowsApi';
import { NODE_CONFIG_FORMS, type NodeConfigFormProps } from '../nodeConfigForms';

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

  it('shows the schedule input when config already has a schedule trigger_kind', () => {
    const { onChange } = renderForm('trigger', { config: { trigger_kind: 'schedule' } });
    const schedule = screen.getByTestId('node-config-trigger-schedule');
    fireEvent.change(schedule, { target: { value: '0 9 * * 1' } });
    expect(onChange).toHaveBeenLastCalledWith({ schedule: '0 9 * * 1' });
  });
});
