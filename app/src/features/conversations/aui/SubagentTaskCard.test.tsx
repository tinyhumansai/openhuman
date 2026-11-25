import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { type SubagentActivity, subagentCancelResolved } from '../../../store/chatRuntimeSlice';
import { SubagentTaskCard } from './SubagentTaskCard';

// The nested transcript's MessagePrimitive needs a ThreadPrimitive.Viewport.
// These tests exercise the card and its reply actions without mounting a thread.
vi.mock('../../../components/assistant-ui/elements/task-card.aui', () => ({
  TaskTranscript: () => <div>Nested transcript</div>,
}));

const append = vi.hoisted(() => vi.fn());
// The enclosing (parent) message's parts, as `MarkdownText` reads them.
const parentParts = vi.hoisted(() => ({ value: [] as unknown[] }));
vi.mock('@assistant-ui/react', async importActual => ({
  ...(await importActual<typeof import('@assistant-ui/react')>()),
  useAui: () => ({ thread: { append } }),
  useAuiState: (selector: (state: unknown) => unknown) =>
    selector({ message: { parts: parentParts.value } }),
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
        result={
          {
            status: 'error',
            activity: { ...activity, status: 'failed', output: 'Validation failed.' },
          } as never
        }
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
    // A failure's output stays visible on the card.
    expect(screen.getByText('Validation failed.')).toBeInTheDocument();
  });

  it('keeps a completed report collapsed and ignores legacy worktree fields', () => {
    const legacyActivity: SubagentActivity & {
      worktreePath: string;
      changedFiles: string[];
      isDirty: boolean;
    } = {
      ...activity,
      status: 'success',
      output: 'Research is complete.',
      worktreePath: '/repo/.claude/worktrees/worker',
      changedFiles: ['notes.md'],
      isDirty: true,
    };
    render(
      <SubagentTaskCard
        type="tool-call"
        toolName="task"
        toolCallId="sub-1"
        args={{} as never}
        argsText="{}"
        result={{ status: 'complete', activity: legacyActivity } as never}
        status={{ type: 'complete' }}
        addResult={() => {}}
        resume={() => {}}
        respondToApproval={async () => {}}
      />
    );

    expect(screen.getByTestId('assistant-ui-subagent-call')).toHaveAttribute(
      'data-status',
      'success'
    );
    expect(screen.getByRole('button', { name: /Delegated to Researcher/i })).toHaveAttribute(
      'aria-expanded',
      'false'
    );
    expect(screen.queryByTestId('subagent-report')).toBeNull();
    expect(screen.queryByText('Research is complete.')).toBeNull();
    expect(screen.queryByTestId('worktree-actions')).toBeNull();
    expect(screen.queryByText(/worker/)).toBeNull();
  });

  it('toggles the completed report open and closed, rendered as markdown', async () => {
    render(
      <SubagentTaskCard
        type="tool-call"
        toolName="task"
        toolCallId="sub-report"
        args={{} as never}
        argsText="{}"
        result={
          {
            status: 'complete',
            activity: {
              ...activity,
              status: 'success',
              output: '**Answer:** saved the flow.\n\n- trigger updated\n- step 2 shortened',
            },
          } as never
        }
        status={{ type: 'complete' }}
        addResult={() => {}}
        resume={() => {}}
        respondToApproval={async () => {}}
      />
    );

    const header = screen.getByRole('button', { name: /Delegated to Researcher/i });
    fireEvent.click(header);
    expect(header).toHaveAttribute('aria-expanded', 'true');

    const report = await screen.findByTestId('subagent-report');
    const strong = await within(report).findByText('Answer:');
    expect(strong.tagName).toBe('STRONG');
    expect(
      within(report)
        .getAllByRole('listitem')
        .map(li => li.textContent)
    ).toEqual(['trigger updated', 'step 2 shortened']);
    expect(report.textContent).not.toContain('**');

    // The same button collapses it again.
    fireEvent.click(header);
    expect(header).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByTestId('subagent-report')).toBeNull();

    fireEvent.click(header);
    expect(header).toHaveAttribute('aria-expanded', 'true');
    expect(await screen.findByTestId('subagent-report')).toBeInTheDocument();
  });

  it("leaves the report's [n] markers as text, not the parent turn's citations", async () => {
    parentParts.value = [
      { type: 'source', sourceType: 'url', id: 's1', url: 'https://parent.example/a' },
    ];
    try {
      render(
        <SubagentTaskCard
          type="tool-call"
          toolName="task"
          toolCallId="sub-citations"
          args={{} as never}
          argsText="{}"
          result={
            {
              status: 'complete',
              activity: { ...activity, status: 'success', output: 'Checked the docs [1].' },
            } as never
          }
          status={{ type: 'complete' }}
          addResult={() => {}}
          resume={() => {}}
          respondToApproval={async () => {}}
        />
      );
      fireEvent.click(screen.getByRole('button', { name: /Delegated to Researcher/i }));

      const report = await screen.findByTestId('subagent-report');
      await waitFor(() => expect(report.textContent).toContain('Checked the docs [1].'));
      expect(within(report).queryByRole('button')).toBeNull();
      expect(report.querySelector('a')).toBeNull();
    } finally {
      parentParts.value = [];
    }
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
