import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type {
  SubagentActivity,
  ToolTimelineEntry,
  ToolTimelineEntryStatus,
} from '../../../../store/chatRuntimeSlice';
import {
  type BackgroundProcess,
  BackgroundProcessesPanel,
  selectBackgroundProcesses,
} from '../BackgroundProcessesPanel';

function sub(partial: Partial<SubagentActivity> & { taskId: string }): SubagentActivity {
  return { agentId: 'researcher', toolCalls: [], ...partial };
}

function entry(
  status: ToolTimelineEntryStatus,
  subagent?: SubagentActivity,
  name = 'subagent:researcher'
): ToolTimelineEntry {
  return { id: `e-${subagent?.taskId ?? name}`, name, round: 0, status, subagent };
}

describe('selectBackgroundProcesses', () => {
  it('keeps only detached (mode==="async") sub-agents', () => {
    const timeline: ToolTimelineEntry[] = [
      entry('running', sub({ taskId: 'sub-async', mode: 'async', prompt: 'research towers' })),
      entry('running', sub({ taskId: 'sub-typed', mode: 'typed', prompt: 'inline work' })),
      entry('success', undefined, 'web_search'), // non-subagent tool row
    ];
    const out = selectBackgroundProcesses(timeline);
    expect(out.map(p => p.taskId)).toEqual(['sub-async']);
    expect(out[0].goal).toBe('research towers');
  });

  it('dedupes by taskId and sorts running first', () => {
    const timeline: ToolTimelineEntry[] = [
      entry('success', sub({ taskId: 'sub-done', mode: 'async' })),
      entry('running', sub({ taskId: 'sub-live', mode: 'async' })),
      entry('running', sub({ taskId: 'sub-live', mode: 'async' })), // duplicate row
    ];
    const out = selectBackgroundProcesses(timeline);
    expect(out.map(p => p.taskId)).toEqual(['sub-live', 'sub-done']); // running first, deduped
  });

  it('derives name, tool count and steps', () => {
    const out = selectBackgroundProcesses([
      entry(
        'running',
        sub({
          taskId: 'sub-1',
          mode: 'async',
          displayName: 'Researcher',
          iterations: 3,
          toolCalls: [
            { callId: 'a', toolName: 't', status: 'success' },
            { callId: 'b', toolName: 't', status: 'success' },
          ],
        })
      ),
    ]);
    expect(out[0]).toMatchObject({
      name: 'Researcher',
      toolCount: 2,
      iterations: 3,
      status: 'running',
    });
  });
});

describe('BackgroundProcessesPanel', () => {
  const procs: BackgroundProcess[] = [
    {
      taskId: 'sub-1',
      name: 'Researcher',
      goal: 'research the Eiffel Tower',
      status: 'running',
      toolCount: 16,
    },
    {
      taskId: 'sub-2',
      name: 'Archivist',
      goal: 'summarize notes',
      status: 'success',
      toolCount: 4,
    },
  ];

  it('renders nothing when closed', () => {
    const { container } = render(
      <BackgroundProcessesPanel
        open={false}
        processes={procs}
        onClose={vi.fn()}
        onOpenProcess={vi.fn()}
      />
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('lists processes and opens one on click', async () => {
    const onOpenProcess = vi.fn();
    render(
      <BackgroundProcessesPanel
        open
        processes={procs}
        onClose={vi.fn()}
        onOpenProcess={onOpenProcess}
      />
    );
    const rows = screen.getAllByTestId('background-process-row');
    expect(rows).toHaveLength(2);
    expect(screen.getByText('Researcher')).toBeInTheDocument();
    expect(screen.getByText('research the Eiffel Tower')).toBeInTheDocument();

    await userEvent.click(rows[0]);
    expect(onOpenProcess).toHaveBeenCalledWith('sub-1');
  });

  it('shows an empty state when there are no background tasks', () => {
    render(
      <BackgroundProcessesPanel open processes={[]} onClose={vi.fn()} onOpenProcess={vi.fn()} />
    );
    expect(screen.getByText(/No background tasks in this chat/i)).toBeInTheDocument();
  });

  it('renders every status variant (running / done / failed / needs-you / cancelled)', () => {
    const all: BackgroundProcess[] = [
      { taskId: 'r', name: 'R', goal: 'g', status: 'running', toolCount: 2 },
      { taskId: 'd', name: 'D', goal: 'g', status: 'success', toolCount: 2 },
      { taskId: 'e', name: 'E', goal: 'g', status: 'error', toolCount: 2 },
      { taskId: 'a', name: 'A', goal: 'g', status: 'awaiting_user', toolCount: 2 },
      { taskId: 'c', name: 'C', goal: 'g', status: 'cancelled', toolCount: 2 },
    ];
    render(
      <BackgroundProcessesPanel open processes={all} onClose={vi.fn()} onOpenProcess={vi.fn()} />
    );
    expect(screen.getByText('Running')).toBeInTheDocument();
    expect(screen.getByText('Done')).toBeInTheDocument();
    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(screen.getByText('Needs you')).toBeInTheDocument();
    expect(screen.getByText('Cancelled')).toBeInTheDocument();
  });

  it('renders singular tool-call wording, step count, and suppresses an empty goal', () => {
    const rows: BackgroundProcess[] = [
      {
        taskId: 'g',
        name: 'WithGoal',
        goal: 'investigate the bridge',
        status: 'success',
        toolCount: 4,
      },
      { taskId: 's1', name: 'NoGoal', goal: '', status: 'running', toolCount: 1, iterations: 3 },
    ];
    const { container } = render(
      <BackgroundProcessesPanel open processes={rows} onClose={vi.fn()} onOpenProcess={vi.fn()} />
    );
    expect(container.textContent).toContain('1 tool call'); // singular branch (NoGoal row)
    expect(container.textContent).toContain('3 steps'); // iterations branch
    // The goal renders for the row that has one; the goal-less row adds no copy.
    expect(screen.getAllByText('investigate the bridge')).toHaveLength(1);
  });

  it('closes on Escape', () => {
    const onClose = vi.fn();
    render(
      <BackgroundProcessesPanel open processes={procs} onClose={onClose} onOpenProcess={vi.fn()} />
    );
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('renders the broader activity sections (cron + memory) alongside sub-agents', () => {
    // Outside Tauri the activity hook is a no-op, so these sections fall back to
    // their empty states — which is exactly what we assert here.
    render(
      <BackgroundProcessesPanel open processes={procs} onClose={vi.fn()} onOpenProcess={vi.fn()} />
    );
    expect(screen.getByText('In this chat')).toBeInTheDocument();
    expect(screen.getByText('Scheduled jobs')).toBeInTheDocument();
    expect(screen.getByText('No scheduled jobs.')).toBeInTheDocument();
    expect(screen.getByText('Memory syncing')).toBeInTheDocument();
    expect(screen.getByText('All memories up to date')).toBeInTheDocument();
  });
});
