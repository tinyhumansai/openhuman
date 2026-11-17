import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { TaskCardDock, TaskCardDockProvider } from '../lib/task-card-dock';
import { TaskCard } from './task-card';

vi.mock('@/lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
describe('task card conversation placement', () => {
  it('keeps active work at the conversation end and releases completed work', () => {
    const { container, rerender } = render(
      <TaskCardDockProvider>
        <div data-testid="original">
          <TaskCard label="Inspect UI" state="working" />
        </div>
        <TaskCardDock />
      </TaskCardDockProvider>
    );
    expect(container.querySelector('[data-slot=task-card-dock]')).not.toHaveClass(
      'sticky',
      'fixed',
      'absolute'
    );
    expect(container.querySelector('[data-slot=task-card]')?.parentElement).toHaveAttribute(
      'data-slot',
      'task-card-dock'
    );
    rerender(
      <TaskCardDockProvider>
        <div data-testid="original">
          <TaskCard label="Inspect UI" state="done" />
        </div>
        <TaskCardDock />
      </TaskCardDockProvider>
    );
    expect(container.querySelector('[data-slot=task-card]')?.parentElement).toHaveAttribute(
      'data-testid',
      'original'
    );
    expect(container.querySelector('[data-slot=task-card-dock]')).toBeEmptyDOMElement();
  });
});
