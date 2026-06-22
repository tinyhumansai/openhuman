import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { worktreeApi } from '../../services/api/worktreeApi';
import { revealPath } from '../../utils/openUrl';
import WorktreeActions from './WorktreeActions';

vi.mock('../../services/api/worktreeApi', () => ({
  worktreeApi: { diff: vi.fn(), remove: vi.fn() },
}));
vi.mock('../../utils/openUrl', () => ({ revealPath: vi.fn().mockResolvedValue(undefined) }));
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

const mockDiff = vi.mocked(worktreeApi.diff);
const mockRemove = vi.mocked(worktreeApi.remove);
const mockReveal = vi.mocked(revealPath);

const PATH = '/r/.claude/worktrees/worker-a';

describe('WorktreeActions', () => {
  beforeEach(() => {
    mockDiff.mockReset();
    mockRemove.mockReset().mockResolvedValue(true);
    mockReveal.mockReset().mockResolvedValue(undefined);
  });

  it('open reveals the path in the OS file manager', async () => {
    const user = userEvent.setup();
    render(<WorktreeActions path={PATH} />);
    await user.click(screen.getByTestId('worktree-open'));
    expect(mockReveal).toHaveBeenCalledWith(PATH);
  });

  it('diff fetches once and renders the summary, toggling closed on second click', async () => {
    mockDiff.mockResolvedValue(' src/a.rs | 2 +-');
    const user = userEvent.setup();
    render(<WorktreeActions path={PATH} />);
    await user.click(screen.getByTestId('worktree-diff'));
    await waitFor(() =>
      expect(screen.getByTestId('worktree-diff-output')).toHaveTextContent('src/a.rs')
    );
    expect(mockDiff).toHaveBeenCalledTimes(1);
    await user.click(screen.getByTestId('worktree-diff'));
    expect(screen.queryByTestId('worktree-diff-output')).toBeNull();
  });

  it('removes a clean worktree in one click (force=false)', async () => {
    const onRemoved = vi.fn();
    const user = userEvent.setup();
    render(<WorktreeActions path={PATH} isDirty={false} onRemoved={onRemoved} />);
    await user.click(screen.getByTestId('worktree-remove'));
    await waitFor(() => expect(mockRemove).toHaveBeenCalledWith(PATH, false));
    expect(onRemoved).toHaveBeenCalledWith(PATH);
  });

  it('a dirty worktree requires explicit confirmation before removal', async () => {
    const onRemoved = vi.fn();
    const user = userEvent.setup();
    render(<WorktreeActions path={PATH} isDirty onRemoved={onRemoved} />);
    await user.click(screen.getByTestId('worktree-remove'));
    // No removal yet — a confirm row appears instead.
    expect(mockRemove).not.toHaveBeenCalled();
    expect(screen.getByTestId('worktree-remove-confirm')).toBeInTheDocument();
    // Discard & remove forces.
    await user.click(screen.getByTestId('worktree-remove-confirm-yes'));
    await waitFor(() => expect(mockRemove).toHaveBeenCalledWith(PATH, true));
    expect(onRemoved).toHaveBeenCalledWith(PATH);
  });

  it('resets the removing state after success when no onRemoved parent drops the row', async () => {
    // Inline timeline use (ToolTimelineBlock) renders without onRemoved, so the
    // component is never unmounted by a parent — it must reset its own state or
    // the button stays stuck on "worktree.removing" with disabled forever.
    const user = userEvent.setup();
    render(<WorktreeActions path={PATH} isDirty={false} />);
    await user.click(screen.getByTestId('worktree-remove'));
    await waitFor(() => expect(mockRemove).toHaveBeenCalledWith(PATH, false));
    await waitFor(() => {
      const btn = screen.getByTestId('worktree-remove');
      expect(btn).toHaveTextContent('worktree.action.remove');
      expect(btn).not.toBeDisabled();
    });
  });

  it('preserve cancels the dirty-removal confirm without removing', async () => {
    const user = userEvent.setup();
    render(<WorktreeActions path={PATH} isDirty />);
    await user.click(screen.getByTestId('worktree-remove'));
    await user.click(screen.getByTestId('worktree-preserve'));
    expect(mockRemove).not.toHaveBeenCalled();
    expect(screen.queryByTestId('worktree-remove-confirm')).toBeNull();
    expect(screen.getByTestId('worktree-remove')).toBeInTheDocument();
  });
});
