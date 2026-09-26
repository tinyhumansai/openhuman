import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { type SubagentActivity, subagentCancelResolved } from '../../../store/chatRuntimeSlice';
import { SubagentTaskCard } from './SubagentTaskCard';

// The nested transcript's MessagePrimitive needs a ThreadPrimitive.Viewport.
// These tests exercise the card and its reply actions without mounting a thread.
vi.mock('../../../components/assistant-ui/elements/task-card.aui', () => ({
  TaskTranscript: () => <div>Nested transcript</div>,
}));

const append = vi.hoisted(() => vi.fn());
vi.mock('@assistant-ui/react', async importActual => ({
  ...(await importActual<typeof import('@assistant-ui/react')>()),
  useAui: () => ({ thread: { append } }),
}));

const dispatch = vi.hoisted(() => vi.fn());
vi.mock('../../../store/hooks', () => ({ useAppDispatch: () => dispatch }));

const cancel = vi.hoisted(() => vi.fn());
vi.mock('../../../services/api/subagentApi', () => ({ subagentApi: { cancel } }));

const activity: SubagentActivity = {
  taskId: 'sub-1',
  agentId: 'researcher',
  displayName: 'Researcher',
  toolCalls: [],
  transcript: [{ kind: 'thinking', text: 'Checking primary sources.' }],
};

describe('SubagentTaskCard', () => {
  it('renders a running delegation, collapsed, with a nested-transcript disclosure', () => {
    render(
      <SubagentTaskCard
        type="tool-call"
        toolName="task"
        toolCallId="sub-1"
        args={{ progress: activity } as never}
        argsText="{}"
        result={undefined}
        status={{ type: 'running' }}
        addResult={() => {}}
        resume={() => {}}
        respondToApproval={async () => {}}
      />
    );

    expect(screen.getByTestId('assistant-ui-subagent-call')).toHaveAttribute(
      'data-state',
      'working'
    );
    expect(screen.getByText('Delegated to Researcher')).toBeInTheDocument();
    // The transcript is collapsed by default, but the card knows it has one
    // (the vendored `TaskCard`'s disclosure chevron only renders when
    // `children` is non-empty).
    expect(screen.queryByText('Checking primary sources.')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Delegated to Researcher/i })).toHaveAttribute(
      'aria-expanded',
      'false'
    );
  });

  it('renders a failed delegation as failed, not as a completed one', () => {
    render(
      <SubagentTaskCard
        type="tool-call"
        toolName="task"
        toolCallId="sub-1"
        args={{} as never}
        argsText="{}"
        result={{ status: 'error', activity: { ...activity, status: 'failed' } } as never}
        status={{ type: 'complete' }}
        addResult={() => {}}
        resume={() => {}}
        respondToApproval={async () => {}}
      />
    );

    expect(screen.getByTestId('assistant-ui-subagent-call')).toHaveAttribute(
      'data-status',
      'failed'
    );
  });

  it('renders the awaiting-user reply box and lets the answer through the composer path', async () => {
    render(
      <SubagentTaskCard
        type="tool-call"
        toolName="task"
        toolCallId="sub-1"
        args={
          {
            progress: { ...activity, status: 'awaiting_user', awaitingQuestion: 'Which repo?' },
          } as never
        }
        argsText="{}"
        result={undefined}
        messages={[]}
        status={{ type: 'requires-action', reason: 'interrupt' }}
        addResult={() => {}}
        resume={() => {}}
        respondToApproval={async () => {}}
      />
    );

    expect(screen.getByTestId('subagent-awaiting-user')).toBeInTheDocument();
    expect(screen.getByTestId('subagent-awaiting-question')).toHaveTextContent('Which repo?');
    fireEvent.change(screen.getByTestId('subagent-answer-input'), {
      target: { value: 'OpenHuman' },
    });
    fireEvent.click(screen.getByTestId('subagent-answer-send'));
    expect(append).toHaveBeenCalledWith({
      role: 'user',
      content: [{ type: 'text', text: 'OpenHuman' }],
    });
    expect(screen.getByTestId('subagent-answer-sent')).toBeInTheDocument();
  });

  // The card has no other signal for a run the core aborted (the aborted task
  // never reports) or one that already ended: the cancel answer must settle it,
  // or the spinner and a dead Cancel button stay forever.
  it.each([
    { cancelled: true, outcome: undefined },
    { cancelled: false, outcome: 'failed' as const },
  ])(
    'settles the card from the core cancel answer (cancelled=$cancelled)',
    async ({ cancelled, outcome }) => {
      dispatch.mockClear();
      cancel.mockResolvedValueOnce({ cancelled, taskId: 'sub-1', outcome });
      render(
        <SubagentTaskCard
          type="tool-call"
          toolName="task"
          toolCallId="sub-1"
          args={{ progress: activity } as never}
          argsText="{}"
          result={undefined}
          status={{ type: 'running' }}
          addResult={() => {}}
          resume={() => {}}
          respondToApproval={async () => {}}
        />
      );

      fireEvent.click(screen.getByTestId('subagent-cancel-task'));
      await waitFor(() =>
        expect(dispatch).toHaveBeenCalledWith(
          subagentCancelResolved({ taskId: 'sub-1', cancelled, outcome })
        )
      );
      expect(cancel).toHaveBeenCalledWith('sub-1');
    }
  );
});
