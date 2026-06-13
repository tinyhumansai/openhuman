import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { agentWorkApi, type AgentWorkResponse } from '../../services/api/agentWorkApi';
import IntelligenceAgentWorkTab from './IntelligenceAgentWorkTab';

vi.mock('../../services/api/agentWorkApi', () => ({ agentWorkApi: { list: vi.fn() } }));

// i18n → echo the key so assertions can target stable strings.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

// Navigation + store: the tab only dispatches + navigates on click; stub them.
vi.mock('react-router-dom', () => ({ useNavigate: () => vi.fn() }));
vi.mock('../../store/hooks', () => ({ useAppDispatch: () => vi.fn() }));
vi.mock('../../store/threadSlice', () => ({
  loadThreadMessages: vi.fn(),
  loadThreads: vi.fn(),
  setSelectedThread: vi.fn(),
}));

const mockList = vi.mocked(agentWorkApi.list);

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
});
