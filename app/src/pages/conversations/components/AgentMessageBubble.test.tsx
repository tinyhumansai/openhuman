import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { BubbleMarkdown, TableCellMarkdown } from './AgentMessageBubble';

const mocks = vi.hoisted(() => ({ openUrl: vi.fn(), openWorkspacePath: vi.fn() }));

vi.mock('../../../utils/openUrl', () => ({ openUrl: mocks.openUrl }));

vi.mock('../../../utils/tauriCommands/workspacePaths', () => ({
  openWorkspacePath: mocks.openWorkspacePath,
}));

describe('AgentMessageBubble markdown links', () => {
  beforeEach(() => {
    mocks.openUrl.mockReset();
    mocks.openUrl.mockResolvedValue(undefined);
    mocks.openWorkspacePath.mockReset();
    mocks.openWorkspacePath.mockResolvedValue(undefined);
  });

  test('opens allowed external links through the OS URL handler', async () => {
    render(<BubbleMarkdown content="[docs](https://example.com/docs)" />);

    await userEvent.click(screen.getByRole('link', { name: 'docs' }));

    await waitFor(() => expect(mocks.openUrl).toHaveBeenCalledWith('https://example.com/docs'));
    expect(mocks.openWorkspacePath).not.toHaveBeenCalled();
  });

  test('opens workspace links through the Tauri workspace path command', async () => {
    render(<BubbleMarkdown content="[summary](workspace:memory_tree/content/summary.md)" />);

    await userEvent.click(screen.getByRole('link', { name: 'summary' }));

    await waitFor(() =>
      expect(mocks.openWorkspacePath).toHaveBeenCalledWith('memory_tree/content/summary.md')
    );
    expect(mocks.openUrl).not.toHaveBeenCalled();
  });

  test('logs workspace link open failures for diagnostics', async () => {
    const error = new Error('missing file');
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    mocks.openWorkspacePath.mockRejectedValueOnce(error);

    try {
      render(<BubbleMarkdown content="[summary](workspace:memory_tree/content/missing.md)" />);

      await userEvent.click(screen.getByRole('link', { name: 'summary' }));

      await waitFor(() =>
        expect(consoleError).toHaveBeenCalledWith('workspace open failed:', error)
      );
    } finally {
      consoleError.mockRestore();
    }
  });

  test('uses the same workspace link handling inside table cells', async () => {
    render(<TableCellMarkdown content="[note](openhuman-workspace:/docs/note.md)" />);

    await userEvent.click(screen.getByRole('link', { name: 'note' }));

    await waitFor(() => expect(mocks.openWorkspacePath).toHaveBeenCalledWith('docs/note.md'));
    expect(mocks.openUrl).not.toHaveBeenCalled();
  });

  test('does not open raw file links from markdown', async () => {
    render(<BubbleMarkdown content="[secret](file:///etc/passwd)" />);

    await userEvent.click(screen.getByText('secret'));

    expect(mocks.openUrl).not.toHaveBeenCalled();
    expect(mocks.openWorkspacePath).not.toHaveBeenCalled();
  });
});
