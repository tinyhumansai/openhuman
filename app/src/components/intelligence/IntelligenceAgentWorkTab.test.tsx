import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  agentWorkApi,
  type AgentWorkResponse,
  type AgentWorkRow,
} from '../../services/api/agentWorkApi';
import IntelligenceAgentWorkTab from './IntelligenceAgentWorkTab';

const hoisted = vi.hoisted(() => ({ dispatch: vi.fn(), navigate: vi.fn() }));

vi.mock('../../services/api/agentWorkApi', () => ({
  agentWorkApi: { list: vi.fn(), control: vi.fn() },
}));

// i18n → echo the key so assertions can target stable strings.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

// Navigation + store: the tab only dispatches + navigates on click; stub them.
vi.mock('react-router-dom', () => ({ useNavigate: () => hoisted.navigate }));
vi.mock('../../store/hooks', () => ({ useAppDispatch: () => hoisted.dispatch }));
vi.mock('../../store/threadSlice', () => ({
  loadThreadMessages: vi.fn(),
  loadThreads: vi.fn(),
  setSelectedThread: vi.fn(),
}));

const mockList = vi.mocked(agentWorkApi.list);
const mockControl = vi.mocked(agentWorkApi.control);

/** Build a single-row response in a given bucket/status for control tests. */
function singleRowResponse(bucket: AgentWorkRow['bucket'], status: string): AgentWorkResponse {
  const row: AgentWorkRow = {
    runId: 'run-1',
    kind: 'subagent',
    displayName: 'Researcher',
    bucket,
    status,
    startedAt: '2026-01-01T00:00:00Z',
    updatedAt: '2026-01-01T00:01:00Z',
    inputTokens: 0,
    outputTokens: 0,
    costUsd: 0,
    toolCount: 0,
  };
  return {
    total: 1,
    groups: (['needs_input', 'working', 'completed', 'failed', 'stopped'] as const).map(b => ({
      bucket: b,
      count: b === bucket ? 1 : 0,
      rows: b === bucket ? [row] : [],
    })),
  };
}

function emptyResponse(): AgentWorkResponse {
  return {
    total: 0,
    groups: (['needs_input', 'working', 'completed', 'failed', 'stopped'] as const).map(bucket => ({
      bucket,
      count: 0,
      rows: [],
    })),
  };
}

function workingResponse(): AgentWorkResponse {
  return {
    total: 1,
    groups: [
      { bucket: 'needs_input', count: 0, rows: [] },
      {
        bucket: 'working',
        count: 1,
        rows: [
          {
            runId: 'run-1',
            kind: 'subagent',
            agentId: 'agent-a',
            displayName: 'Researcher',
            bucket: 'working',
            status: 'running',
            workerThreadId: 'thread-w',
            startedAt: '2026-01-01T00:00:00Z',
            updatedAt: '2026-01-01T00:01:00Z',
            elapsedMs: 60000,
            inputTokens: 1200,
            outputTokens: 300,
            costUsd: 0.05,
            toolCount: 3,
          },
        ],
      },
      { bucket: 'completed', count: 0, rows: [] },
      { bucket: 'failed', count: 0, rows: [] },
      { bucket: 'stopped', count: 0, rows: [] },
    ],
  };
}

describe('IntelligenceAgentWorkTab', () => {
  beforeEach(() => {
    // Reset the queue + implementation so a prior test's resolve/reject can't
    // leak via the mount setTimeout into the next render (clearMocks only wipes
    // call history, not queued *Once values / persistent implementations).
    mockList.mockReset();
    mockControl.mockReset();
    hoisted.dispatch.mockReset();
    hoisted.navigate.mockReset();
  });

  it('fetches agent work on mount', async () => {
    mockList.mockResolvedValue(emptyResponse());
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(1));
  });

  it('shows the loading state before the RPC resolves', () => {
    mockList.mockReturnValue(new Promise(() => {}));
    render(<IntelligenceAgentWorkTab />);
    expect(screen.getByText('intelligence.agentWork.loading')).toBeInTheDocument();
  });

  it('shows the error box when the RPC rejects', async () => {
    mockList.mockRejectedValue(new Error('boom'));
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() =>
      expect(screen.getByText(/intelligence\.agentWork\.failedToLoad/)).toBeInTheDocument()
    );
    expect(screen.getByText(/boom/)).toBeInTheDocument();
  });

  it('shows the empty state when total is 0', async () => {
    mockList.mockResolvedValue(emptyResponse());
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() =>
      expect(screen.getByText('intelligence.agentWork.empty')).toBeInTheDocument()
    );
  });

  it('renders a grouped working row with its display name and bucket label', async () => {
    mockList.mockResolvedValue(workingResponse());
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());
    expect(screen.getByText('intelligence.agentWork.bucket.working')).toBeInTheDocument();
    // 1200 + 300 input/output tokens → "1.5K"
    expect(screen.getByText('1.5K')).toBeInTheDocument();
    // $0.05 cost formatted
    expect(screen.getByText('$0.05')).toBeInTheDocument();
    // worker-thread jump button present
    expect(screen.getByText('intelligence.agentWork.openWorker')).toBeInTheDocument();
  });

  it('opens worker threads on the routed chat URL', async () => {
    mockList.mockResolvedValue(workingResponse());
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    fireEvent.click(screen.getByText('intelligence.agentWork.openWorker'));

    expect(hoisted.navigate).toHaveBeenCalledWith('/chat/thread-w');
  });

  it('shows Stop (not Retry/Continue) for a live working run', async () => {
    mockList.mockResolvedValue(singleRowResponse('working', 'running'));
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());
    expect(screen.getByText('intelligence.agentWork.action.stop')).toBeInTheDocument();
    expect(screen.queryByText('intelligence.agentWork.action.retry')).not.toBeInTheDocument();
    expect(screen.queryByText('intelligence.agentWork.action.continue')).not.toBeInTheDocument();
    // Follow-up is always available.
    expect(screen.getByText('intelligence.agentWork.action.followUp')).toBeInTheDocument();
  });

  it('shows Retry (not Stop) for a failed run', async () => {
    mockList.mockResolvedValue(singleRowResponse('failed', 'failed'));
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());
    expect(screen.getByText('intelligence.agentWork.action.retry')).toBeInTheDocument();
    expect(screen.queryByText('intelligence.agentWork.action.stop')).not.toBeInTheDocument();
  });

  it('stop fires the control RPC and refetches', async () => {
    mockList.mockResolvedValue(singleRowResponse('working', 'running'));
    mockControl.mockResolvedValue(singleRowResponse('stopped', 'cancelled').groups[4].rows[0]);
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    fireEvent.click(screen.getByText('intelligence.agentWork.action.stop'));
    await waitFor(() =>
      expect(mockControl).toHaveBeenCalledWith({
        runId: 'run-1',
        action: 'stop',
        message: undefined,
      })
    );
    // The success path refetches the list (mount call + post-control call).
    await waitFor(() => expect(mockList).toHaveBeenCalledTimes(2));
  });

  it('continue opens a composer and sends the typed message', async () => {
    mockList.mockResolvedValue(singleRowResponse('needs_input', 'awaiting_user'));
    mockControl.mockResolvedValue(singleRowResponse('working', 'running').groups[1].rows[0]);
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    // Open the continue composer.
    fireEvent.click(screen.getByText('intelligence.agentWork.action.continue'));
    const textarea = screen.getByLabelText('intelligence.agentWork.action.continuePlaceholder');
    // Send is disabled until there is text.
    const send = screen.getByText('intelligence.agentWork.action.send');
    expect(send).toBeDisabled();

    fireEvent.change(textarea, { target: { value: 'use staging' } });
    expect(send).not.toBeDisabled();
    fireEvent.click(send);

    await waitFor(() =>
      expect(mockControl).toHaveBeenCalledWith({
        runId: 'run-1',
        action: 'continue',
        message: 'use staging',
      })
    );
  });

  it('surfaces a control error inline without refetching', async () => {
    mockList.mockResolvedValue(singleRowResponse('working', 'running'));
    mockControl.mockRejectedValue(new Error('nope'));
    render(<IntelligenceAgentWorkTab />);
    await waitFor(() => expect(screen.getByText('Researcher')).toBeInTheDocument());

    fireEvent.click(screen.getByText('intelligence.agentWork.action.stop'));
    await waitFor(() =>
      expect(screen.getByText(/intelligence\.agentWork\.action\.failed/)).toBeInTheDocument()
    );
    expect(screen.getByText(/nope/)).toBeInTheDocument();
    // Only the mount fetch — the failed control must not refetch.
    expect(mockList).toHaveBeenCalledTimes(1);
  });
});
