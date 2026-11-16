/**
 * Behavior tests for the flows "Tool" node's native tool picker. The options
 * come from `agentRegistryApi.availableTools` (`openhuman.agent_registry_available_tools`,
 * the native registry the flow engine resolves `oh:<name>` slugs against);
 * picks are stored as `oh:<name>`, and a failed fetch falls back to a raw
 * text input. `useT()` falls back to the bundled English map with no provider
 * mounted (same convention as the sibling `nodeConfigForms.test.tsx`).
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { NativeToolField } from '../nativeToolFields';

const availableToolsMock = vi.hoisted(() => vi.fn());
vi.mock('../../../../../services/api/agentRegistryApi', () => ({
  agentRegistryApi: { availableTools: availableToolsMock },
}));

const TOOLS = [
  { name: 'file_read', description: 'Read a file from the workspace.' },
  { name: 'web_search', description: 'Search the web.' },
];

function renderField(value = '') {
  const onChange = vi.fn();
  render(<NativeToolField label="Tool" value={value} onChange={onChange} testId="tool-slug" />);
  return { onChange };
}

async function optionValues(): Promise<string[]> {
  const select = screen.getByTestId('tool-slug');
  await waitFor(() => expect(select).not.toBeDisabled());
  return within(select)
    .getAllByRole('option')
    .map(o => o.getAttribute('value') ?? '');
}

describe('NativeToolField', () => {
  beforeEach(() => {
    availableToolsMock.mockReset();
  });

  it('lists the native tools from the agent registry', async () => {
    availableToolsMock.mockResolvedValue(TOOLS);
    renderField();
    expect(await optionValues()).toEqual(['', 'file_read', 'web_search']);
    expect(availableToolsMock).toHaveBeenCalledTimes(1);
  });

  it('stores the pick as an oh:-prefixed slug and clears to empty', async () => {
    availableToolsMock.mockResolvedValue(TOOLS);
    const { onChange } = renderField();
    await optionValues();
    const select = screen.getByTestId('tool-slug');
    fireEvent.change(select, { target: { value: 'web_search' } });
    expect(onChange).toHaveBeenLastCalledWith('oh:web_search');
    fireEvent.change(select, { target: { value: '' } });
    expect(onChange).toHaveBeenLastCalledWith('');
  });

  it("shows the selected tool's description", async () => {
    availableToolsMock.mockResolvedValue(TOOLS);
    renderField('oh:web_search');
    expect(await screen.findByTestId('node-config-native-desc')).toHaveTextContent(
      'Search the web.'
    );
    expect(screen.getByTestId('tool-slug')).toHaveValue('web_search');
  });

  it('keeps a saved tool that the registry no longer lists selectable', async () => {
    availableToolsMock.mockResolvedValue(TOOLS);
    renderField('oh:retired_tool');
    expect(await optionValues()).toEqual(['', 'retired_tool', 'file_read', 'web_search']);
    expect(screen.queryByTestId('node-config-native-desc')).not.toBeInTheDocument();
  });

  it('falls back to a raw text input when the registry fetch fails', async () => {
    availableToolsMock.mockRejectedValue(new Error('rpc down'));
    const { onChange } = renderField('oh:web_search');
    const input = await screen.findByTestId('tool-slug-custom');
    expect(input).toHaveValue('web_search');
    fireEvent.change(input, { target: { value: ' shell ' } });
    expect(onChange).toHaveBeenLastCalledWith('oh:shell');
    fireEvent.change(input, { target: { value: '' } });
    expect(onChange).toHaveBeenLastCalledWith('');
  });
});
