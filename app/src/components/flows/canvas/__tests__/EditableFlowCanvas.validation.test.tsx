/**
 * EditableFlowCanvas — validation UX (Phase 3c) + draft/dirty state (Phase 3d).
 *
 * Drives the canvas through the public `FlowCanvas editable` entry point with a
 * mocked `flowsApi` so `validateFlow` is deterministic. Covers:
 *  - an invalid graph shows the inline error banner, rings the offending node,
 *    and blocks Save;
 *  - a valid-with-warnings graph surfaces warnings distinctly and allows Save;
 *  - dirty tracking gates Save/Discard, Discard resets to baseline, and a
 *    successful Save clears the dirty flag.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowNode } from '../../../../lib/flows/graphAdapter';
import FlowCanvas from '../FlowCanvas';

const validateFlow = vi.hoisted(() => vi.fn());
const listFlowConnections = vi.hoisted(() => vi.fn());
vi.mock('../../../../services/api/flowsApi', () => ({ validateFlow, listFlowConnections }));

function triggerNode(): FlowNode {
  return {
    id: 't',
    type: 'flowNode',
    position: { x: 0, y: 0 },
    data: {
      kind: 'trigger',
      name: 'Start',
      config: {},
      ports: [],
      inputPorts: ['main'],
      outputPorts: ['main'],
    },
  };
}

const META = { schema_version: 1, id: 'wf_1', name: 'My flow' } as const;

function renderCanvas(props: Partial<React.ComponentProps<typeof FlowCanvas>> = {}) {
  return render(
    <FlowCanvas
      editable
      nodes={[triggerNode()]}
      edges={[]}
      meta={META}
      onSave={vi.fn().mockResolvedValue(undefined)}
      {...props}
    />
  );
}

describe('EditableFlowCanvas — validation + dirty state', () => {
  beforeEach(() => {
    validateFlow.mockReset();
    listFlowConnections.mockReset();
    listFlowConnections.mockResolvedValue([]);
  });

  it('surfaces hard errors, rings the offending node, and blocks Save', async () => {
    validateFlow.mockResolvedValue({
      valid: false,
      errors: ['invalid config for node t: missing schedule'],
      warnings: [],
    });
    const { container } = renderCanvas();

    // Make an edit so the graph is dirty (Save is only ever enabled when dirty).
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    // Validation runs automatically on the debounce after the edit (the manual
    // Validate button now lives on the selected node card).

    const errors = await screen.findByTestId('flow-editor-errors');
    expect(errors).toHaveTextContent('invalid config for node t: missing schedule');

    // Save is blocked while there are hard errors, even though the graph is dirty.
    expect(screen.getByTestId('flow-editor-save')).toBeDisabled();

    // The named node ('t') is ringed with the error class on its RF wrapper.
    await waitFor(() =>
      expect(container.querySelector('.react-flow__node[data-id="t"]')).toHaveClass(
        'flow-node-error'
      )
    );
  });

  it('shows warnings distinctly from errors and allows Save', async () => {
    validateFlow.mockResolvedValue({
      valid: true,
      errors: [],
      warnings: ['this trigger kind does not fire automatically yet'],
    });
    renderCanvas();

    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    // Auto-validation (debounced) surfaces the warning.

    const warnings = await screen.findByTestId('flow-editor-warnings');
    expect(warnings).toHaveTextContent('does not fire automatically');
    // A valid graph never renders the errors list…
    expect(screen.queryByTestId('flow-editor-errors')).not.toBeInTheDocument();
    // …and Save is allowed (warnings don't block).
    expect(screen.getByTestId('flow-editor-save')).not.toBeDisabled();
  });

  it('tracks dirty state: Save/Discard gate on it, Discard resets, Save clears it', async () => {
    validateFlow.mockResolvedValue({ valid: true, errors: [], warnings: [] });
    const onSave = vi.fn().mockResolvedValue(undefined);
    const onDirtyChange = vi.fn();
    renderCanvas({ onSave, onDirtyChange });

    // Pristine: no dirty badge, Save + Discard disabled.
    expect(screen.queryByTestId('flow-editor-dirty')).not.toBeInTheDocument();
    expect(screen.getByTestId('flow-editor-save')).toBeDisabled();
    expect(screen.getByTestId('flow-editor-discard')).toBeDisabled();

    // Edit → dirty.
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    expect(screen.getByTestId('flow-editor-dirty')).toBeInTheDocument();
    expect(screen.getByTestId('flow-editor-save')).not.toBeDisabled();
    expect(screen.getByTestId('flow-editor-discard')).not.toBeDisabled();
    expect(onDirtyChange).toHaveBeenLastCalledWith(true);
    expect(screen.getAllByTestId('flow-node')).toHaveLength(2);

    // Discard → back to the single trigger, no longer dirty.
    fireEvent.click(screen.getByTestId('flow-editor-discard'));
    expect(screen.getAllByTestId('flow-node')).toHaveLength(1);
    expect(screen.queryByTestId('flow-editor-dirty')).not.toBeInTheDocument();
    expect(onDirtyChange).toHaveBeenLastCalledWith(false);

    // Edit again and Save → onSave called, dirty cleared once it resolves.
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    fireEvent.click(screen.getByTestId('flow-editor-save'));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    const graph = onSave.mock.calls[0][0];
    expect(graph.nodes.map((n: { kind: string }) => n.kind).sort()).toEqual(['agent', 'trigger']);
    await waitFor(() => expect(screen.queryByTestId('flow-editor-dirty')).not.toBeInTheDocument());
    expect(onDirtyChange).toHaveBeenLastCalledWith(false);
  });

  it('starts dirty when the host passes initialDirty (a remount carrying unsaved content)', async () => {
    validateFlow.mockResolvedValue({ valid: true, errors: [], warnings: [] });
    const onDirtyChange = vi.fn();
    // Mirrors `FlowCanvasPage` remounting the canvas (`key={canvasVersion}`)
    // after accepting a copilot proposal: the incoming nodes/edges ARE the
    // component's "initial" graph, so without `initialDirty` the canvas would
    // seed its baseline from them and instantly read as clean even though
    // nothing was persisted (the P1 this regression test guards against).
    renderCanvas({ onDirtyChange, initialDirty: true });

    expect(screen.getByTestId('flow-editor-dirty')).toBeInTheDocument();
    expect(screen.getByTestId('flow-editor-save')).not.toBeDisabled();
    expect(onDirtyChange).toHaveBeenLastCalledWith(true);
  });

  it('surfaces a Save failure inline and leaves the graph dirty', async () => {
    validateFlow.mockResolvedValue({ valid: true, errors: [], warnings: [] });
    const onSave = vi.fn().mockRejectedValue(new Error('core unreachable'));
    renderCanvas({ onSave });

    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    fireEvent.click(screen.getByTestId('flow-editor-save'));

    const saveError = await screen.findByTestId('flow-editor-save-error');
    expect(saveError).toHaveTextContent('core unreachable');
    // Still dirty — nothing persisted.
    expect(screen.getByTestId('flow-editor-dirty')).toBeInTheDocument();
  });
});
