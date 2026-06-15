import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { worktreeApi, type WorktreeListView } from '../../services/api/worktreeApi';
import IntelligenceWorktreesTab from './IntelligenceWorktreesTab';

vi.mock('../../services/api/worktreeApi', () => ({
  worktreeApi: { list: vi.fn(), diff: vi.fn(), remove: vi.fn() },
}));
vi.mock('../../utils/openUrl', () => ({ revealPath: vi.fn().mockResolvedValue(undefined) }));
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const mockList = vi.mocked(worktreeApi.list);

function view(overrides?: Partial<WorktreeListView>): WorktreeListView {
  return {
    worktrees: [
      {
        path: '/r/.claude/worktrees/worker-a',
        branch: 'worker/a',
        isDirty: true,
        changedFiles: ['src/lib.rs', 'a.rs'],
      },
    ],
    overlaps: [],
    ...overrides,
  };
}

describe('IntelligenceWorktreesTab', () => {
  beforeEach(() => {
    mockList.mockReset();
  });

  it('renders empty state when there are no worktrees', async () => {
    mockList.mockResolvedValue({ worktrees: [], overlaps: [] });
    render(<IntelligenceWorktreesTab />);
    await waitFor(() => expect(screen.getByText('worktree.panel.empty')).toBeInTheDocument());
  });

  it('renders a worktree row with branch, dirty badge and changed files', async () => {
    mockList.mockResolvedValue(view());
    render(<IntelligenceWorktreesTab />);
    await waitFor(() => expect(screen.getByTestId('worktree-row')).toBeInTheDocument());
    expect(screen.getByText('worker/a')).toBeInTheDocument();
    expect(screen.getByText('worktree.dirty')).toBeInTheDocument();
    expect(screen.getByText('src/lib.rs')).toBeInTheDocument();
    expect(screen.getByTestId('worktree-actions')).toBeInTheDocument();
  });

  it('renders an overlap banner when workers touched the same file', async () => {
    mockList.mockResolvedValue(
      view({ overlaps: [{ file: 'src/lib.rs', branches: ['worker/a', 'worker/b'] }] })
    );
    render(<IntelligenceWorktreesTab />);
    await waitFor(() => expect(screen.getByTestId('worktree-overlaps')).toBeInTheDocument());
    expect(screen.getByText('worktree.panel.overlapsTitle')).toBeInTheDocument();
  });

  it('surfaces a load error', async () => {
    mockList.mockRejectedValue(new Error('boom'));
    render(<IntelligenceWorktreesTab />);
    await waitFor(() =>
      expect(screen.getByText(/worktree.panel.failedToLoad/)).toBeInTheDocument()
    );
  });
});
