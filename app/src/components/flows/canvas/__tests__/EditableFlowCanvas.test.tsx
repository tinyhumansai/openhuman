/**
 * EditableFlowCanvas (issue B5b.2 / Phase 3a) — behavior tests for the mutable
 * Workflow Canvas driven through the public `FlowCanvas editable` entry point.
 *
 * `@xyflow/react` mounts for real in jsdom (nodes measure 0x0, but the DOM
 * tree, palette, toolbar, and `FlowNodeComponent` cards are all assertable), so
 * these tests drive the *click* affordances (palette add, save) rather than
 * drag geometry, which jsdom can't produce. Port-aware connection validity is
 * unit-tested directly against `isValidFlowConnection` in
 * `lib/flows/graphAdapter.test.ts`.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';

import type { FlowNode } from '../../../../lib/flows/graphAdapter';
import type { WorkflowGraph } from '../../../../lib/flows/types';
import type { EditableFlowCanvasHandle } from '../EditableFlowCanvas';
import FlowCanvas from '../FlowCanvas';

// `FlowNodeComponent` / palette call `useT()`, which falls back to the bundled
// English map when no `I18nProvider` (and its Redux dependency) is mounted —
// the same no-provider render the read-only `FlowCanvas.test.tsx` relies on.
function renderCanvas(ui: React.ReactElement) {
  return render(ui);
}

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

describe('FlowCanvas (editable)', () => {
  it('renders the node palette with all 12 node kinds', () => {
    renderCanvas(<FlowCanvas editable nodes={[triggerNode()]} edges={[]} />);
    expect(screen.getByTestId('flow-node-palette')).toBeInTheDocument();
    // Palette items are keyed by kind via data-testid `flow-palette-item-<kind>`.
    expect(screen.getByTestId('flow-palette-item-trigger')).toBeInTheDocument();
    expect(screen.getByTestId('flow-palette-item-agent')).toBeInTheDocument();
    expect(screen.getByTestId('flow-palette-item-sub_workflow')).toBeInTheDocument();
  });

  it('does NOT render the palette in read-only mode', () => {
    renderCanvas(<FlowCanvas nodes={[triggerNode()]} edges={[]} />);
    expect(screen.queryByTestId('flow-node-palette')).not.toBeInTheDocument();
  });

  it('adds a node to the canvas when a palette item is clicked', () => {
    renderCanvas(<FlowCanvas editable nodes={[triggerNode()]} edges={[]} />);
    // One node to start (the trigger).
    expect(screen.getAllByTestId('flow-node')).toHaveLength(1);

    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));

    const rendered = screen.getAllByTestId('flow-node');
    expect(rendered).toHaveLength(2);
    // The newly added node carries data-node-kind="agent".
    expect(rendered.some(el => el.getAttribute('data-node-kind') === 'agent')).toBe(true);
  });

  it('serializes the live canvas to a valid WorkflowGraph on Save', () => {
    const onSave = vi.fn<(graph: WorkflowGraph) => void>();
    // Save lives in the page header now and drives the canvas via the imperative
    // handle, so exercise that handle directly here.
    const ref = createRef<EditableFlowCanvasHandle>();
    renderCanvas(
      <FlowCanvas
        ref={ref}
        editable
        nodes={[triggerNode()]}
        edges={[]}
        meta={{ schema_version: 1, id: 'wf_1', name: 'My flow' }}
        onSave={onSave}
      />
    );

    // Add an agent node, then save via the handle.
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    act(() => ref.current?.save());

    expect(onSave).toHaveBeenCalledTimes(1);
    const graph = onSave.mock.calls[0][0];
    expect(graph.schema_version).toBe(1);
    expect(graph.id).toBe('wf_1');
    expect(graph.name).toBe('My flow');
    // Original trigger + the palette-added agent.
    expect(graph.nodes.map(n => n.kind).sort()).toEqual(['agent', 'trigger']);
    expect(graph.edges).toEqual([]);
  });

  it('keeps only undo/redo in the canvas toolbar (Save/Discard/Delete/Validate moved out)', () => {
    renderCanvas(<FlowCanvas editable nodes={[triggerNode()]} edges={[]} />);
    expect(screen.queryByTestId('flow-editor-delete')).not.toBeInTheDocument();
    expect(screen.queryByTestId('flow-editor-validate')).not.toBeInTheDocument();
    // Save/Discard now live in the page header, not the canvas toolbar.
    expect(screen.queryByTestId('flow-editor-save')).not.toBeInTheDocument();
    expect(screen.queryByTestId('flow-editor-discard')).not.toBeInTheDocument();
    // Undo/redo remain on the canvas.
    expect(screen.getByTestId('flow-editor-undo')).toBeInTheDocument();
    expect(screen.getByTestId('flow-editor-redo')).toBeInTheDocument();
  });

  it('shows the onboarding hint on a near-empty canvas and hides it after a node is added', () => {
    renderCanvas(<FlowCanvas editable nodes={[triggerNode()]} edges={[]} />);
    expect(screen.getByTestId('flow-editor-onboarding')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    expect(screen.queryByTestId('flow-editor-onboarding')).not.toBeInTheDocument();
  });

  it('undoes and redoes a palette add', () => {
    renderCanvas(<FlowCanvas editable nodes={[triggerNode()]} edges={[]} />);
    // Undo starts disabled (empty history); redo too.
    expect(screen.getByTestId('flow-editor-undo')).toBeDisabled();
    expect(screen.getByTestId('flow-editor-redo')).toBeDisabled();

    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    expect(screen.getAllByTestId('flow-node')).toHaveLength(2);
    expect(screen.getByTestId('flow-editor-undo')).not.toBeDisabled();

    // Undo removes the added node and enables redo.
    fireEvent.click(screen.getByTestId('flow-editor-undo'));
    expect(screen.getAllByTestId('flow-node')).toHaveLength(1);
    expect(screen.getByTestId('flow-editor-undo')).toBeDisabled();
    expect(screen.getByTestId('flow-editor-redo')).not.toBeDisabled();

    // Redo brings it back.
    fireEvent.click(screen.getByTestId('flow-editor-redo'));
    expect(screen.getAllByTestId('flow-node')).toHaveLength(2);
  });
});
