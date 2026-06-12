import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

// i18n stub — return the key verbatim so we can assert on label keys.
vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

// The task-board surface is heavy (Redux + RPC); stub it to a sentinel so the
// panel test stays focused on the re-homing chrome.
vi.mock('../../../intelligence/IntelligenceTasksTab', () => ({
  default: () => <div data-testid="intelligence-tasks-tab" />,
}));

const navigateBack = vi.fn();
vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    breadcrumbs: [{ label: 'Settings' }, { label: 'Developer Options' }],
  }),
}));

const TasksPanel = (await import('../TasksPanel')).default;

function renderPanel() {
  return render(
    <MemoryRouter>
      <TasksPanel />
    </MemoryRouter>
  );
}

describe('TasksPanel', () => {
  it('renders the panel shell, title, description and the task board', () => {
    renderPanel();
    expect(screen.getByTestId('tasks-panel')).toBeInTheDocument();
    // SettingsHeader title + the descriptive blurb both come from i18n keys.
    expect(screen.getAllByText('memory.tab.tasks').length).toBeGreaterThan(0);
    expect(screen.getByText('memory.tab.tasksDescription')).toBeInTheDocument();
    // The re-homed task board is mounted unchanged.
    expect(screen.getByTestId('intelligence-tasks-tab')).toBeInTheDocument();
  });
});
