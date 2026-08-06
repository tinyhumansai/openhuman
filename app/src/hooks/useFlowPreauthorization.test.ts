import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { preauthorizeFlow } from '../services/api/approvalApi';
import {
  type ApprovalManifest,
  getApprovalManifest,
  setFlowEnabled,
} from '../services/api/flowsApi';
import { useFlowPreauthorization } from './useFlowPreauthorization';

vi.mock('../services/api/approvalApi', () => ({ preauthorizeFlow: vi.fn() }));
vi.mock('../services/api/flowsApi', () => ({
  getApprovalManifest: vi.fn(),
  setFlowEnabled: vi.fn(),
}));

const mockManifest = (overrides: Partial<ApprovalManifest>): ApprovalManifest => ({
  entries: [],
  missing: [],
  already_trusted: [],
  gate_installed: true,
  ...overrides,
});

const MISSING_TWO = mockManifest({
  entries: [
    { kind: 'approvable', node_id: 'n1', tool_name: 'flows_http_request', label: 'Call API' },
    { kind: 'approvable', node_id: 'n2', tool_name: 'GMAIL_SEND_EMAIL', label: 'Send email' },
  ],
  missing: ['flows_http_request', 'GMAIL_SEND_EMAIL'],
});

describe('useFlowPreauthorization', () => {
  beforeEach(() => {
    vi.mocked(getApprovalManifest).mockReset();
    vi.mocked(setFlowEnabled)
      .mockReset()
      .mockResolvedValue({} as never);
    vi.mocked(preauthorizeFlow)
      .mockReset()
      .mockResolvedValue({
        flow_id: 'flow-1',
        granted: [],
        already_trusted: [],
        gate_installed: true,
      });
  });

  it('beginEnable enables directly when no grants are missing', async () => {
    vi.mocked(getApprovalManifest).mockResolvedValue(mockManifest({}));
    const onSettled = vi.fn();
    const { result } = renderHook(() => useFlowPreauthorization({ onSettled }));

    let enabledNow = false;
    await act(async () => {
      enabledNow = await result.current.beginEnable('flow-1');
    });

    expect(enabledNow).toBe(true);
    expect(setFlowEnabled).toHaveBeenCalledWith('flow-1', true);
    expect(result.current.pending).toBeNull();
    expect(onSettled).toHaveBeenCalledWith('no-card', 'flow-1');
  });

  it('beginEnable enables directly when the approval gate is not installed', async () => {
    vi.mocked(getApprovalManifest).mockResolvedValue(
      mockManifest({ gate_installed: false, missing: [] })
    );
    const { result } = renderHook(() => useFlowPreauthorization());

    let enabledNow = false;
    await act(async () => {
      enabledNow = await result.current.beginEnable('flow-1');
    });

    expect(enabledNow).toBe(true);
    expect(setFlowEnabled).toHaveBeenCalledWith('flow-1', true);
  });

  it('beginEnable fails open when the manifest RPC errors', async () => {
    vi.mocked(getApprovalManifest).mockRejectedValue(new Error('boom'));
    const { result } = renderHook(() => useFlowPreauthorization());

    let enabledNow = false;
    await act(async () => {
      enabledNow = await result.current.beginEnable('flow-1');
    });

    // A broken manifest RPC must never make a flow impossible to enable.
    expect(enabledNow).toBe(true);
    expect(setFlowEnabled).toHaveBeenCalledWith('flow-1', true);
  });

  it('beginEnable surfaces the card instead of enabling when grants are missing', async () => {
    vi.mocked(getApprovalManifest).mockResolvedValue(MISSING_TWO);
    const { result } = renderHook(() => useFlowPreauthorization());

    let enabledNow = true;
    await act(async () => {
      enabledNow = await result.current.beginEnable('flow-1');
    });

    expect(enabledNow).toBe(false);
    expect(setFlowEnabled).not.toHaveBeenCalled();
    expect(result.current.pending).toMatchObject({ flowId: 'flow-1', enableOnApprove: true });
  });

  it('approveAll grants the missing tools, then enables, then settles approved', async () => {
    vi.mocked(getApprovalManifest).mockResolvedValue(MISSING_TWO);
    const onSettled = vi.fn();
    const { result } = renderHook(() => useFlowPreauthorization({ onSettled }));

    await act(async () => {
      await result.current.beginEnable('flow-1');
    });
    await act(async () => {
      await result.current.approveAll();
    });

    expect(preauthorizeFlow).toHaveBeenCalledWith('flow-1', [
      'flows_http_request',
      'GMAIL_SEND_EMAIL',
    ]);
    expect(setFlowEnabled).toHaveBeenCalledWith('flow-1', true);
    // Grant-then-enable ordering: trust must exist before the flow is live.
    expect(vi.mocked(preauthorizeFlow).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(setFlowEnabled).mock.invocationCallOrder[0]
    );
    await waitFor(() => expect(result.current.pending).toBeNull());
    expect(onSettled).toHaveBeenCalledWith('approved', 'flow-1');
  });

  it('deny turns the flow off and settles denied', async () => {
    vi.mocked(getApprovalManifest).mockResolvedValue(MISSING_TWO);
    const onSettled = vi.fn();
    const { result } = renderHook(() => useFlowPreauthorization({ onSettled }));

    await act(async () => {
      await result.current.beginEnable('flow-1');
    });
    await act(async () => {
      await result.current.deny();
    });

    expect(preauthorizeFlow).not.toHaveBeenCalled();
    expect(setFlowEnabled).toHaveBeenCalledWith('flow-1', false);
    await waitFor(() => expect(result.current.pending).toBeNull());
    expect(onSettled).toHaveBeenCalledWith('denied', 'flow-1');
  });

  it('approveAll failure keeps the card up with an error key', async () => {
    vi.mocked(getApprovalManifest).mockResolvedValue(MISSING_TWO);
    vi.mocked(preauthorizeFlow).mockRejectedValue(new Error('rpc down'));
    const onSettled = vi.fn();
    const { result } = renderHook(() => useFlowPreauthorization({ onSettled }));

    await act(async () => {
      await result.current.beginEnable('flow-1');
    });
    await act(async () => {
      await result.current.approveAll();
    });

    expect(result.current.pending).not.toBeNull();
    expect(result.current.errorKey).toBe('flows.enableApproval.error');
    expect(setFlowEnabled).not.toHaveBeenCalled();
    expect(onSettled).not.toHaveBeenCalled();
  });

  it('blocked-only manifests still surface the card, and approve enables without granting', async () => {
    // Readonly tier: nothing approvable, but the user must see the Block at
    // save time instead of a silently-enabled flow whose runs then fail.
    vi.mocked(getApprovalManifest).mockResolvedValue(
      mockManifest({
        entries: [
          { kind: 'blocked', node_id: 'n1', tool_name: 'flows_http_request', label: 'Call API' },
        ],
        missing: [],
      })
    );
    const onSettled = vi.fn();
    const { result } = renderHook(() => useFlowPreauthorization({ onSettled }));

    let enabledNow = true;
    await act(async () => {
      enabledNow = await result.current.beginEnable('flow-1');
    });
    expect(enabledNow).toBe(false);
    expect(result.current.pending).not.toBeNull();
    expect(setFlowEnabled).not.toHaveBeenCalled();

    await act(async () => {
      await result.current.approveAll();
    });
    // Nothing to grant on a blocked-only card ("Enable anyway").
    expect(preauthorizeFlow).not.toHaveBeenCalled();
    expect(setFlowEnabled).toHaveBeenCalledWith('flow-1', true);
    expect(onSettled).toHaveBeenCalledWith('approved', 'flow-1');
  });

  it('checkAfterSave skips the card for a disabled flow', async () => {
    const { result } = renderHook(() => useFlowPreauthorization());

    let shown = true;
    await act(async () => {
      shown = await result.current.checkAfterSave('flow-1', false);
    });

    expect(shown).toBe(false);
    expect(getApprovalManifest).not.toHaveBeenCalled();
  });

  it('checkAfterSave skips the card (not the save) when the manifest RPC errors', async () => {
    vi.mocked(getApprovalManifest).mockRejectedValue(new Error('manifest down'));
    const { result } = renderHook(() => useFlowPreauthorization());

    let shown = true;
    await act(async () => {
      shown = await result.current.checkAfterSave('flow-1', true);
    });

    // The flow is already saved and enabled; a broken manifest must not
    // block or revert that — the runtime gate still parks per-node.
    expect(shown).toBe(false);
    expect(result.current.pending).toBeNull();
  });

  it('deny failure keeps the card up with an error key', async () => {
    vi.mocked(getApprovalManifest).mockResolvedValue(MISSING_TWO);
    vi.mocked(setFlowEnabled).mockRejectedValue(new Error('disable failed'));
    const onSettled = vi.fn();
    const { result } = renderHook(() => useFlowPreauthorization({ onSettled }));

    await act(async () => {
      await result.current.beginEnable('flow-1');
    });
    await act(async () => {
      await result.current.deny();
    });

    expect(result.current.pending).not.toBeNull();
    expect(result.current.errorKey).toBe('flows.enableApproval.error');
    expect(onSettled).not.toHaveBeenCalled();
  });

  it('checkAfterSave surfaces a no-enable card for an already-enabled flow', async () => {
    vi.mocked(getApprovalManifest).mockResolvedValue(MISSING_TWO);
    const { result } = renderHook(() => useFlowPreauthorization());

    let shown = false;
    await act(async () => {
      shown = await result.current.checkAfterSave('flow-1', true);
    });

    expect(shown).toBe(true);
    expect(result.current.pending).toMatchObject({ flowId: 'flow-1', enableOnApprove: false });

    // Approve keeps it on WITHOUT a redundant enable call.
    await act(async () => {
      await result.current.approveAll();
    });
    expect(setFlowEnabled).not.toHaveBeenCalled();
  });
});
