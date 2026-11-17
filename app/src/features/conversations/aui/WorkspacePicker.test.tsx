import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { RECENT_FOLDER_LIMIT, recentWorkingFolders, WorkspacePicker } from './WorkspacePicker';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

function at(day: number) {
  return new Date(2026, 9, day, 12).toISOString();
}

describe('recentWorkingFolders', () => {
  it('lists distinct folders newest first, skipping the excluded ones', () => {
    const threads = [
      { actionDir: '/p/a', lastMessageAt: at(1) },
      { actionDir: '/p/b', lastMessageAt: at(3) },
      { actionDir: '/p/a', lastMessageAt: at(4) },
      { actionDir: null, lastMessageAt: at(5) },
      { actionDir: '/p/default', lastMessageAt: at(6) },
    ];
    expect(recentWorkingFolders(threads, ['/p/default', null])).toEqual(['/p/a', '/p/b']);
  });

  it('caps the list', () => {
    const threads = Array.from({ length: RECENT_FOLDER_LIMIT + 3 }, (_, i) => ({
      actionDir: `/p/${i}`,
      lastMessageAt: at(i + 1),
    }));
    expect(recentWorkingFolders(threads, [])).toHaveLength(RECENT_FOLDER_LIMIT);
  });
});

describe('WorkspacePicker', () => {
  it('labels the default folder by name when no folder is bound', () => {
    render(
      <WorkspacePicker value={null} defaultDir="/home/me/projects" recent={[]} onChange={vi.fn()} />
    );
    expect(screen.getByTestId('composer-workspace-label')).toHaveTextContent(
      'composer.workspace.defaultWithName'
    );
  });

  it('shows the bound folder basename and the error when one is passed', () => {
    render(
      <WorkspacePicker
        value="/home/me/projects/site"
        defaultDir={null}
        recent={[]}
        onChange={vi.fn()}
        error="nope"
      />
    );
    expect(screen.getByTestId('composer-workspace-label')).toHaveTextContent('site');
    expect(screen.getByRole('alert')).toHaveTextContent('nope');
  });

  it('binds a recent folder, the default, or opens the chooser', () => {
    const onChange = vi.fn();
    const onChooseFolder = vi.fn();
    render(
      <WorkspacePicker
        value={null}
        defaultDir="/home/me/projects"
        recent={['/home/me/work/api']}
        onChange={onChange}
        onChooseFolder={onChooseFolder}
      />
    );
    const open = () =>
      fireEvent.pointerDown(screen.getByRole('button', { name: 'composer.workspace.label' }), {
        button: 0,
        ctrlKey: false,
      });

    open();
    fireEvent.click(screen.getByTestId('composer-workspace-recent'));
    expect(onChange).toHaveBeenLastCalledWith('/home/me/work/api');

    open();
    fireEvent.click(screen.getByTestId('composer-workspace-default'));
    expect(onChange).toHaveBeenLastCalledWith(null);

    open();
    fireEvent.click(screen.getByTestId('composer-workspace-choose'));
    expect(onChooseFolder).toHaveBeenCalledTimes(1);
  });

  it('hides the chooser where no host can open one', () => {
    render(<WorkspacePicker value={null} defaultDir={null} recent={[]} onChange={vi.fn()} />);
    fireEvent.pointerDown(screen.getByRole('button', { name: 'composer.workspace.label' }), {
      button: 0,
      ctrlKey: false,
    });
    expect(screen.queryByTestId('composer-workspace-choose')).toBeNull();
  });
});
