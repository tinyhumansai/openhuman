import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  buildWorkflow,
  discoverWorkflows,
  dismissSuggestion,
  type FlowSuggestion,
  getFlowRun,
  listAllFlowRuns,
  listFlowRuns,
  listFlows,
  listSuggestions,
  markSuggestionBuilt,
  resumeFlow,
  runFlow,
  setFlowEnabled,
} from './flowsApi';

const mockCallCoreRpc = vi.fn();
vi.mock('../coreRpcClient', () => ({ callCoreRpc: (...a: unknown[]) => mockCallCoreRpc(...a) }));

/** Every `flows_*` handler wraps its payload via `RpcOutcome::single_log`. */
function cliEnvelope<T>(
  result: T,
  logs: string[] = ['did something']
): { result: T; logs: string[] } {
  return { result, logs };
}

describe('flowsApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  describe('resumeFlow', () => {
    it('calls openhuman.flows_resume with id, thread_id, approvals', async () => {
      mockCallCoreRpc.mockResolvedValue(
        cliEnvelope({ output: { nodes: {} }, pending_approvals: [], thread_id: 't1' })
      );

      const result = await resumeFlow('flow-1', 't1', ['node-a']);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_resume',
        params: { id: 'flow-1', thread_id: 't1', approvals: ['node-a'] },
        // flows_resume can run ~600s server-side, so the client budget is raised.
        timeoutMs: 610_000,
      });
      expect(result).toEqual({ output: { nodes: {} }, pending_approvals: [], thread_id: 't1' });
    });

    it('unwraps the { result, logs } envelope', async () => {
      const payload = { output: null, pending_approvals: ['node-b'], thread_id: 't2' };
      mockCallCoreRpc.mockResolvedValue(cliEnvelope(payload));

      const result = await resumeFlow('flow-1', 't2', ['node-b']);

      expect(result).toEqual(payload);
    });

    it('passes through a bare (unwrapped) payload unchanged', async () => {
      const payload = { output: null, pending_approvals: [], thread_id: 't3' };
      mockCallCoreRpc.mockResolvedValue(payload);

      const result = await resumeFlow('flow-1', 't3', []);

      expect(result).toEqual(payload);
    });

    it('propagates rejection from callCoreRpc', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('no pending approval matches'));

      await expect(resumeFlow('flow-1', 't1', ['wrong-node'])).rejects.toThrow(
        'no pending approval matches'
      );
    });
  });

  describe('listFlowRuns', () => {
    it('calls openhuman.flows_list_runs with id', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope([]));

      await listFlowRuns('flow-1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_list_runs',
        params: { id: 'flow-1' },
      });
    });

    it('passes limit when provided', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope([]));

      await listFlowRuns('flow-1', 5);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_list_runs',
        params: { id: 'flow-1', limit: 5 },
      });
    });

    it('unwraps the { result, logs } envelope into the run array', async () => {
      const runs = [
        {
          id: 't1',
          flow_id: 'flow-1',
          thread_id: 't1',
          status: 'completed' as const,
          started_at: '2026-01-01T00:00:00Z',
          finished_at: '2026-01-01T00:01:00Z',
          steps: [],
          pending_approvals: [],
          error: null,
        },
      ];
      mockCallCoreRpc.mockResolvedValue(cliEnvelope(runs));

      const result = await listFlowRuns('flow-1');

      expect(result).toEqual(runs);
    });

    it('propagates rejection from callCoreRpc', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('boom'));

      await expect(listFlowRuns('flow-1')).rejects.toThrow('boom');
    });
  });

  describe('listAllFlowRuns', () => {
    it('calls openhuman.flows_list_all_runs with no params by default', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope([]));

      await listAllFlowRuns();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_list_all_runs',
        params: {},
      });
    });

    it('passes limit when provided and unwraps the envelope', async () => {
      const runs = [
        {
          id: 't1',
          flow_id: 'flow-1',
          thread_id: 't1',
          status: 'failed' as const,
          started_at: '2026-01-01T00:00:00Z',
          finished_at: null,
          steps: [],
          pending_approvals: [],
          error: 'boom',
        },
      ];
      mockCallCoreRpc.mockResolvedValue(cliEnvelope(runs));

      const result = await listAllFlowRuns(50);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_list_all_runs',
        params: { limit: 50 },
      });
      expect(result).toEqual(runs);
    });

    it('propagates rejection from callCoreRpc', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('rpc down'));

      await expect(listAllFlowRuns()).rejects.toThrow('rpc down');
    });
  });

  describe('getFlowRun', () => {
    it('calls openhuman.flows_get_run with run_id', async () => {
      const run = {
        id: 't1',
        flow_id: 'flow-1',
        thread_id: 't1',
        status: 'pending_approval' as const,
        started_at: '2026-01-01T00:00:00Z',
        finished_at: null,
        steps: [{ node_id: 'n1', output: { ok: true } }],
        pending_approvals: ['n2'],
        error: null,
      };
      mockCallCoreRpc.mockResolvedValue(cliEnvelope(run));

      const result = await getFlowRun('t1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_get_run',
        params: { run_id: 't1' },
      });
      expect(result).toEqual(run);
    });

    it('propagates rejection from callCoreRpc', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('flow run not found'));

      await expect(getFlowRun('missing')).rejects.toThrow('flow run not found');
    });
  });

  describe('listFlows', () => {
    const flow = {
      id: 'flow-1',
      name: 'Demo flow',
      enabled: true,
      graph: { nodes: [], edges: [] },
      created_at: '2026-01-01T00:00:00Z',
      updated_at: '2026-01-01T00:00:00Z',
      last_run_at: null,
      last_status: null,
      require_approval: false,
    };

    it('calls openhuman.flows_list with no params', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope([flow]));

      await listFlows();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.flows_list', params: {} });
    });

    it('unwraps the { result, logs } envelope into the flow array', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope([flow]));

      const result = await listFlows();

      expect(result).toEqual([flow]);
    });

    it('propagates rejection from callCoreRpc', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('boom'));

      await expect(listFlows()).rejects.toThrow('boom');
    });
  });

  describe('setFlowEnabled', () => {
    it('calls openhuman.flows_set_enabled with id and enabled', async () => {
      const flow = {
        id: 'flow-1',
        name: 'Demo flow',
        enabled: false,
        graph: {},
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
        last_run_at: null,
        last_status: null,
        require_approval: false,
      };
      mockCallCoreRpc.mockResolvedValue(cliEnvelope(flow));

      const result = await setFlowEnabled('flow-1', false);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_set_enabled',
        params: { id: 'flow-1', enabled: false },
      });
      expect(result).toEqual(flow);
    });

    it('propagates rejection from callCoreRpc', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('flow not found'));

      await expect(setFlowEnabled('missing', true)).rejects.toThrow('flow not found');
    });
  });

  describe('runFlow', () => {
    it('calls openhuman.flows_run with id, input, and the extended timeout', async () => {
      mockCallCoreRpc.mockResolvedValue(
        cliEnvelope({ output: { nodes: {} }, pending_approvals: [], thread_id: 't1' })
      );

      const result = await runFlow('flow-1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_run',
        params: { id: 'flow-1', input: null },
        timeoutMs: 610_000,
      });
      expect(result).toEqual({ output: { nodes: {} }, pending_approvals: [], thread_id: 't1' });
    });

    it('passes a supplied input payload through', async () => {
      mockCallCoreRpc.mockResolvedValue(
        cliEnvelope({ output: null, pending_approvals: [], thread_id: 't2' })
      );

      await runFlow('flow-1', { trigger: 'manual' });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_run',
        params: { id: 'flow-1', input: { trigger: 'manual' } },
        timeoutMs: 610_000,
      });
    });

    it('unwraps the { result, logs } envelope', async () => {
      const payload = { output: null, pending_approvals: ['node-a'], thread_id: 't3' };
      mockCallCoreRpc.mockResolvedValue(cliEnvelope(payload));

      const result = await runFlow('flow-1');

      expect(result).toEqual(payload);
    });

    it('propagates rejection from callCoreRpc', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('flow disabled'));

      await expect(runFlow('flow-1')).rejects.toThrow('flow disabled');
    });
  });

  describe('Flow Scout suggestions', () => {
    const suggestion: FlowSuggestion = {
      id: 'sug_1',
      title: 'Auto-file receipts',
      one_liner: 'Add each Gmail receipt to your sheet.',
      rationale: 'You forward receipts weekly.',
      trigger_hint: 'app_event',
      steps_outline: ['Watch Gmail', 'Append row'],
      suggested_connections: ['composio:gmail:c1'],
      suggested_slugs: ['GMAIL_NEW_GMAIL_MESSAGE'],
      build_prompt: 'Build a workflow that…',
      confidence: 0.8,
      status: 'new',
      created_at: '2026-07-05T00:00:00Z',
      source_run_id: null,
    };

    it('discoverWorkflows calls flows_discover with the extended timeout', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope([suggestion]));

      const result = await discoverWorkflows();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_discover',
        params: {},
        timeoutMs: 610_000,
      });
      expect(result).toEqual([suggestion]);
    });

    it('discoverWorkflows passes thread_id when a chat thread is given', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope([suggestion]));

      await discoverWorkflows('scout-thread-1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_discover',
        params: { thread_id: 'scout-thread-1' },
        timeoutMs: 610_000,
      });
    });

    it('listSuggestions omits status when not provided', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope([]));

      await listSuggestions();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_list_suggestions',
        params: {},
      });
    });

    it('listSuggestions passes the status filter', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope([suggestion]));

      const result = await listSuggestions('new');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_list_suggestions',
        params: { status: 'new' },
      });
      expect(result).toEqual([suggestion]);
    });

    it('dismissSuggestion returns the dismissed flag', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope({ id: 'sug_1', dismissed: true }));

      const result = await dismissSuggestion('sug_1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_dismiss_suggestion',
        params: { id: 'sug_1' },
      });
      expect(result).toBe(true);
    });

    it('markSuggestionBuilt returns the built flag', async () => {
      mockCallCoreRpc.mockResolvedValue(cliEnvelope({ id: 'sug_1', built: true }));

      const result = await markSuggestionBuilt('sug_1');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_mark_suggestion_built',
        params: { id: 'sug_1' },
      });
      expect(result).toBe(true);
    });

    it('propagates rejection from callCoreRpc', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('boom'));

      await expect(discoverWorkflows()).rejects.toThrow('boom');
    });
  });

  describe('buildWorkflow', () => {
    const proposalPayload = {
      type: 'workflow_proposal',
      name: 'Digest',
      graph: { schema_version: 1, name: 'g', nodes: [], edges: [] },
      require_approval: true,
      summary: { trigger: 'manual', steps: [] },
    };

    it('calls flows_build with the structured request and no thread_id when omitted', async () => {
      mockCallCoreRpc.mockResolvedValue(
        cliEnvelope({ proposal: proposalPayload, assistant_text: 'here you go', error: null })
      );

      const result = await buildWorkflow({ mode: 'create', instruction: 'email me a digest' });

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_build',
        params: {
          mode: 'create',
          instruction: 'email me a digest',
          graph: null,
          flow_id: null,
          run_id: null,
          error: null,
          failing_node_ids: [],
        },
        timeoutMs: 610_000,
      });
      expect(result.assistantText).toBe('here you go');
      expect(result.proposal?.name).toBe('Digest');
      expect(result.error).toBeNull();
    });

    it('threads the chat thread_id into flows_build params when provided', async () => {
      mockCallCoreRpc.mockResolvedValue(
        cliEnvelope({ proposal: null, assistant_text: '', error: null })
      );

      await buildWorkflow({ mode: 'revise', instruction: 'add a Slack step' }, 'builder-thread-9');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.flows_build',
        params: {
          mode: 'revise',
          instruction: 'add a Slack step',
          graph: null,
          flow_id: null,
          run_id: null,
          error: null,
          failing_node_ids: [],
          thread_id: 'builder-thread-9',
        },
        timeoutMs: 610_000,
      });
    });
  });
});
