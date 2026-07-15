/**
 * EditableFlowCanvas — validation UX (Phase 3c) + draft/dirty state (Phase 3d).
 *
 * A canvas-refactor moved the Save / Discard / dirty-badge *buttons* out of
 * this component and up into `FlowCanvasPage`'s header — the canvas now only
 * exposes them through the `EditableFlowCanvasHandle` ref (`save()`/
 * `discard()`) and reports state up via `onSaveMetaChange`
 * (`{ dirty, hasErrors, saving }`), same as `onDirtyChange`. See
 * `FlowCanvasPage.test.tsx` for the header-button + confirm-dialog + RPC
 * integration coverage (clicking `flow-editor-save`/`flow-editor-discard`).
 *
 * This file drives the canvas through the public `FlowCanvas editable` entry
 * point with a mocked `flowsApi` so `validateFlow` is deterministic, and
 * covers what `FlowCanvas`/`EditableFlowCanvas` itself still owns:
 *  - an invalid graph shows the inline error banner, rings the offending
 *    node, and the imperative `save()` handle refuses to fire `onSave`;
 *  - a valid-with-warnings graph surfaces warnings distinctly and still
 *    lets `save()` fire;
 *  - dirty tracking gates `save()`/`discard()`, `discard()` resets to
 *    baseline, and a successful `save()` clears the dirty flag.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { createRef } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowNode } from '../../../../lib/flows/graphAdapter';
import type { EditableFlowCanvasHandle, EditorSaveMeta } from '../EditableFlowCanvas';
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
  const ref = createRef<EditableFlowCanvasHandle>();
  const onSaveMetaChange = vi.fn<(meta: EditorSaveMeta) => void>();
  const utils = render(
    <FlowCanvas
      ref={ref}
      editable
      nodes={[triggerNode()]}
      edges={[]}
      meta={META}
      onSave={vi.fn().mockResolvedValue(undefined)}
      onSaveMetaChange={onSaveMetaChange}
      {...props}
    />
  );
  return { ...utils, ref, onSaveMetaChange };
}

/** Latest `{ dirty, hasErrors, saving }` the canvas reported to its host. */
function lastSaveMeta(onSaveMetaChange: ReturnType<typeof vi.fn<(meta: EditorSaveMeta) => void>>) {
  const calls = onSaveMetaChange.mock.calls;
  return calls[calls.length - 1][0];
}

describe('EditableFlowCanvas — validation + dirty state', () => {
  beforeEach(() => {
    validateFlow.mockReset();
    listFlowConnections.mockReset();
    listFlowConnections.mockResolvedValue([]);
  });

  it('surfaces hard errors, rings the offending node, and blocks save()', async () => {
    validateFlow.mockResolvedValue({
      valid: false,
      errors: ['invalid config for node t: missing schedule'],
      warnings: [],
    });
    const onSave = vi.fn().mockResolvedValue(undefined);
    const { container, ref, onSaveMetaChange } = renderCanvas({ onSave });

    // Make an edit so the graph is dirty (Save is only ever attempted when
    // dirty). Validation runs automatically on the debounce after the edit
    // (the manual Validate button now lives on the selected node card).
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));

    const errors = await screen.findByTestId('flow-editor-errors');
    expect(errors).toHaveTextContent('invalid config for node t: missing schedule');

    // The host header reads `hasErrors` off `onSaveMetaChange` to disable its
    // Save button; the canvas itself also refuses to fire `onSave` through
    // the imperative handle while hard errors exist, even though dirty.
    expect(lastSaveMeta(onSaveMetaChange)).toMatchObject({ dirty: true, hasErrors: true });
    act(() => ref.current?.save());
    expect(onSave).not.toHaveBeenCalled();

    // The named node ('t') is ringed with the error class on its RF wrapper.
    await waitFor(() =>
      expect(container.querySelector('.react-flow__node[data-id="t"]')).toHaveClass(
        'flow-node-error'
      )
    );
  });

  it('shows warnings distinctly from errors and still lets save() fire', async () => {
    validateFlow.mockResolvedValue({
      valid: true,
      errors: [],
      warnings: ['this trigger kind does not fire automatically yet'],
    });
    const onSave = vi.fn().mockResolvedValue(undefined);
    const { ref, onSaveMetaChange } = renderCanvas({ onSave });

    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    // Auto-validation (debounced) surfaces the warning.

    const warnings = await screen.findByTestId('flow-editor-warnings');
    expect(warnings).toHaveTextContent('does not fire automatically');
    // A valid graph never renders the errors list…
    expect(screen.queryByTestId('flow-editor-errors')).not.toBeInTheDocument();
    // …and warnings don't block Save.
    expect(lastSaveMeta(onSaveMetaChange)).toMatchObject({ dirty: true, hasErrors: false });

    act(() => ref.current?.save());
    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it('tracks dirty state: save()/discard() gate on it, discard resets, save clears it', async () => {
    validateFlow.mockResolvedValue({ valid: true, errors: [], warnings: [] });
    const onSave = vi.fn().mockResolvedValue(undefined);
    const onDirtyChange = vi.fn();
    const { ref, onSaveMetaChange } = renderCanvas({ onSave, onDirtyChange });

    // Pristine: not dirty; save()/discard() both no-op through the imperative
    // handle (the host header renders both buttons disabled in this state).
    expect(lastSaveMeta(onSaveMetaChange)).toMatchObject({ dirty: false });
    act(() => ref.current?.discard());
    act(() => ref.current?.save());
    expect(onSave).not.toHaveBeenCalled();

    // Edit → dirty.
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    expect(onDirtyChange).toHaveBeenLastCalledWith(true);
    expect(lastSaveMeta(onSaveMetaChange)).toMatchObject({ dirty: true });
    expect(screen.getAllByTestId('flow-node')).toHaveLength(2);

    // Discard → back to the single trigger, no longer dirty.
    act(() => ref.current?.discard());
    expect(screen.getAllByTestId('flow-node')).toHaveLength(1);
    expect(onDirtyChange).toHaveBeenLastCalledWith(false);
    expect(lastSaveMeta(onSaveMetaChange)).toMatchObject({ dirty: false });

    // Edit again and save() → onSave called, dirty cleared once it resolves.
    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    act(() => ref.current?.save());
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    const graph = onSave.mock.calls[0][0];
    expect(graph.nodes.map((n: { kind: string }) => n.kind).sort()).toEqual(['agent', 'trigger']);
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(false));
    await waitFor(() => expect(lastSaveMeta(onSaveMetaChange)).toMatchObject({ dirty: false }));
  });

  it('starts dirty when the host passes initialDirty (a remount carrying unsaved content)', () => {
    validateFlow.mockResolvedValue({ valid: true, errors: [], warnings: [] });
    const onSave = vi.fn().mockResolvedValue(undefined);
    const onDirtyChange = vi.fn();
    // Mirrors `FlowCanvasPage` remounting the canvas (`key={canvasVersion}`)
    // after accepting a copilot proposal: the incoming nodes/edges ARE the
    // component's "initial" graph, so without `initialDirty` the canvas would
    // seed its baseline from them and instantly read as clean even though
    // nothing was persisted (the P1 this regression test guards against).
    const { ref, onSaveMetaChange } = renderCanvas({ onSave, onDirtyChange, initialDirty: true });

    expect(onDirtyChange).toHaveBeenLastCalledWith(true);
    expect(lastSaveMeta(onSaveMetaChange)).toMatchObject({ dirty: true });
    act(() => ref.current?.save());
    expect(onSave).toHaveBeenCalledTimes(1);
  });

  it('surfaces a Save failure inline and leaves the graph dirty', async () => {
    validateFlow.mockResolvedValue({ valid: true, errors: [], warnings: [] });
    const onSave = vi.fn().mockRejectedValue(new Error('core unreachable'));
    const onDirtyChange = vi.fn();
    const { ref } = renderCanvas({ onSave, onDirtyChange });

    fireEvent.click(screen.getByTestId('flow-palette-item-agent'));
    act(() => ref.current?.save());

    const saveError = await screen.findByTestId('flow-editor-save-error');
    expect(saveError).toHaveTextContent('core unreachable');
    // Still dirty — nothing persisted.
    expect(onDirtyChange).toHaveBeenLastCalledWith(true);
  });
});
