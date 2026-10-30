import { act, render, renderHook, screen } from '@testing-library/react';
import { startTransition, Suspense, useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowGraph } from '../../../../lib/flows/types';
import type { FlowValidation } from '../../../../services/api/flowsApi';
import { useFlowValidation } from '../useFlowValidation';

const validateFlow = vi.hoisted(() => vi.fn());
vi.mock('../../../../services/api/flowsApi', () => ({ validateFlow }));

function graph(name: string): WorkflowGraph {
  return {
    schema_version: 1,
    name,
    nodes: [{ id: 'start', kind: 'trigger', name: 'Start', config: {}, ports: [] }],
    edges: [],
  };
}

const valid: FlowValidation = { valid: true, errors: [], warnings: [] };
const invalid: FlowValidation = { valid: false, errors: ['bad config'], warnings: [] };

describe('useFlowValidation draft revision isolation', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    validateFlow.mockReset();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('does not let a suspended, uncommitted draft invalidate the active result', async () => {
    let finish!: (value: FlowValidation) => void;
    validateFlow.mockImplementationOnce(
      () =>
        new Promise<FlowValidation>(resolve => {
          finish = resolve;
        })
    );
    const pendingForever = new Promise<void>(() => {});
    let transitionToB!: () => void;
    let validateNow!: () => Promise<FlowValidation | null>;

    function Probe() {
      const [revision, setRevision] = useState<'A' | 'B'>('A');
      transitionToB = () => setRevision('B');
      const state = useFlowValidation(graph(revision), revision, false);
      if (revision === 'B') throw pendingForever;
      validateNow = state.validateNow;
      return <div>Committed draft A</div>;
    }

    render(
      <Suspense fallback={<div>Waiting for draft B</div>}>
        <Probe />
      </Suspense>
    );
    let request!: Promise<FlowValidation | null>;
    act(() => {
      request = validateNow();
    });
    act(() => {
      startTransition(transitionToB);
    });
    expect(screen.getByText('Committed draft A')).toBeInTheDocument();

    await act(async () => {
      finish(valid);
      expect(await request).toEqual(valid);
    });
  });

  it('does not let a previous draft RPC validate a newly edited draft before its debounce', async () => {
    let resolveOld!: (value: FlowValidation) => void;
    validateFlow.mockImplementationOnce(
      () =>
        new Promise(resolve => {
          resolveOld = resolve;
        })
    );
    validateFlow.mockResolvedValueOnce(valid);

    const { result, rerender } = renderHook(({ draft, key }) => useFlowValidation(draft, key), {
      initialProps: { draft: graph('old'), key: 'old' },
    });

    let oldRequest!: Promise<FlowValidation | null>;
    act(() => {
      oldRequest = result.current.validateNow();
    });
    expect(result.current.validating).toBe(true);

    rerender({ draft: graph('new'), key: 'new' });
    expect(result.current.validation).toBeNull();
    expect(result.current.validating).toBe(false);

    await act(async () => {
      resolveOld(invalid);
      expect(await oldRequest).toBeNull();
    });
    expect(result.current.validation).toBeNull();

    await act(async () => {
      await result.current.validateNow();
    });
    expect(result.current.validation).toEqual(valid);
    expect(result.current.validating).toBe(false);
  });

  it('uses the latest committed graph when validating after a re-render', async () => {
    validateFlow.mockResolvedValue(valid);
    const { result, rerender } = renderHook(({ draft, key }) => useFlowValidation(draft, key), {
      initialProps: { draft: graph('before'), key: 'before' },
    });
    rerender({ draft: graph('committed'), key: 'committed' });
    await act(async () => {
      await result.current.validateNow();
    });
    expect(validateFlow).toHaveBeenLastCalledWith(expect.objectContaining({ name: 'committed' }));
    expect(result.current.validation).toEqual(valid);
  });

  it('clears already displayed validation synchronously when the draft key changes', async () => {
    validateFlow.mockResolvedValueOnce(invalid);
    const { result, rerender } = renderHook(({ draft, key }) => useFlowValidation(draft, key), {
      initialProps: { draft: graph('A'), key: 'A' },
    });
    await act(async () => {
      await result.current.validateNow();
    });
    expect(result.current.validation).toEqual(invalid);
    rerender({ draft: graph('B'), key: 'B' });
    expect(result.current.validation).toBeNull();
    expect(result.current.validating).toBe(false);
  });

  it('retains the newest result on the same draft when two manual validations resolve backwards', async () => {
    let resolveFirst!: (value: FlowValidation) => void;
    let resolveSecond!: (value: FlowValidation) => void;
    validateFlow.mockImplementationOnce(
      () =>
        new Promise(resolve => {
          resolveFirst = resolve;
        })
    );
    validateFlow.mockImplementationOnce(
      () =>
        new Promise(resolve => {
          resolveSecond = resolve;
        })
    );

    const { result } = renderHook(() => useFlowValidation(graph('A'), 'A'));
    let first!: Promise<FlowValidation | null>;
    let second!: Promise<FlowValidation | null>;
    act(() => {
      first = result.current.validateNow();
      second = result.current.validateNow();
    });
    await act(async () => {
      resolveSecond(valid);
      expect(await second).toEqual(valid);
    });
    await act(async () => {
      resolveFirst(invalid);
      expect(await first).toBeNull();
    });
    expect(result.current.validation).toEqual(valid);
    expect(result.current.validating).toBe(false);
  });
});
