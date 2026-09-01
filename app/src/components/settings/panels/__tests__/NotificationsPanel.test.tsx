import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import notificationReducer, {
  type NotificationCategory,
  type NotificationPreferences,
} from '../../../../store/notificationSlice';
import NotificationsPanel from '../NotificationsPanel';

/**
 * `NotificationsPanel` is the `/settings/notifications` route
 * (`settingsRouteElements.tsx:89`) and had no test of any kind — measured at
 * 13.33% lines by the settings coverage run, which is only the module-level
 * `CATEGORIES` table being constructed on import.
 *
 * Note `pages/__tests__/Notifications.test.tsx` covers the notification *feed*,
 * a different surface; it never renders this panel.
 */

// `t` returns the key, except the aria template, which needs a real `{name}`
// placeholder for the interpolation to be observable at all.
const ARIA_TEMPLATE = 'Toggle {name} notifications';
vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (k: string) => (k === 'settings.notifications.categoryToggleAria' ? ARIA_TEMPLATE : k),
  }),
}));

// The real `SettingsPanel` drags in the router, the settings-route registry and
// the layout context; this panel's only decision about it is the `embedded`
// branch, so a marker stands in for the chrome.
vi.mock('../../layout/SettingsPanel', () => ({
  default: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="settings-panel-chrome">{children}</div>
  ),
}));

/** Every category the panel renders, in render order. */
const CATEGORY_IDS: NotificationCategory[] = [
  'messages',
  'agents',
  'skills',
  'system',
  'meetings',
  'reminders',
  'important',
];

function buildStore(preferences?: Partial<NotificationPreferences>) {
  const store = configureStore({ reducer: { notifications: notificationReducer } });
  if (preferences) {
    // Drive the real reducer rather than hand-building state, so the preloaded
    // shape cannot drift from what the slice actually produces.
    for (const [category, enabled] of Object.entries(preferences)) {
      store.dispatch({ type: 'notifications/setPreference', payload: { category, enabled } });
    }
  }
  return store;
}

function renderPanel(
  opts: { preferences?: Partial<NotificationPreferences>; embedded?: boolean } = {}
) {
  const store = buildStore(opts.preferences);
  return {
    store,
    ...render(
      <Provider store={store}>
        <NotificationsPanel embedded={opts.embedded} />
      </Provider>
    ),
  };
}

const prefs = (store: ReturnType<typeof buildStore>) => store.getState().notifications.preferences;

const switchFor = (id: NotificationCategory) =>
  screen.getByRole('switch', {
    name: ARIA_TEMPLATE.replace('{name}', `settings.notifications.category.${id}.title`),
  });

beforeEach(() => {
  vi.clearAllMocks();
});

describe('NotificationsPanel — rendering', () => {
  it('renders one switch per notification category', () => {
    renderPanel();
    expect(screen.getAllByRole('switch')).toHaveLength(CATEGORY_IDS.length);
  });

  it('renders a title and a description row for every category', () => {
    renderPanel();
    for (const id of CATEGORY_IDS) {
      expect(screen.getByText(`settings.notifications.category.${id}.title`)).toBeInTheDocument();
      expect(screen.getByText(`settings.notifications.category.${id}.desc`)).toBeInTheDocument();
    }
  });

  it('renders the category footer note', () => {
    renderPanel();
    expect(screen.getByText('settings.notifications.categoryFooter')).toBeInTheDocument();
  });

  it('interpolates the category name into each switch aria-label', () => {
    renderPanel();
    // If `.replace('{name}', title)` were dropped, every switch would share the
    // same accessible name and `getByRole` would throw on multiple matches.
    for (const id of CATEGORY_IDS) {
      expect(switchFor(id)).toBeInTheDocument();
    }
  });

  it('associates each label with its switch via htmlFor/id', () => {
    renderPanel();
    for (const id of CATEGORY_IDS) {
      const label = screen.getByText(`settings.notifications.category.${id}.title`);
      expect(label.tagName).toBe('LABEL');
      expect(label).toHaveAttribute('for', `switch-notif-${id}`);
      expect(switchFor(id)).toHaveAttribute('id', `switch-notif-${id}`);
    }
  });
});

describe('NotificationsPanel — reflects store state', () => {
  it('shows every category enabled for the slice defaults', () => {
    renderPanel();
    for (const id of CATEGORY_IDS) {
      expect(switchFor(id)).toHaveAttribute('aria-checked', 'true');
    }
  });

  it('shows a disabled category as unchecked', () => {
    renderPanel({ preferences: { agents: false } });
    expect(switchFor('agents')).toHaveAttribute('aria-checked', 'false');
    // ...and does not disturb its neighbours.
    expect(switchFor('messages')).toHaveAttribute('aria-checked', 'true');
  });

  it('renders a mixed preference set correctly, switch by switch', () => {
    renderPanel({ preferences: { messages: false, skills: false, important: false } });
    const expected: Record<NotificationCategory, string> = {
      messages: 'false',
      agents: 'true',
      skills: 'false',
      system: 'true',
      meetings: 'true',
      reminders: 'true',
      important: 'false',
    };
    for (const id of CATEGORY_IDS) {
      expect(switchFor(id)).toHaveAttribute('aria-checked', expected[id]);
    }
  });
});

describe('NotificationsPanel — toggling', () => {
  it('turns an enabled category OFF', () => {
    const { store } = renderPanel();
    fireEvent.click(switchFor('system'));
    expect(prefs(store).system).toBe(false);
  });

  it('turns a disabled category ON', () => {
    // The other direction of the `!preferences[category]` negation: a handler
    // hardcoded to `false` would pass the test above and fail this one.
    const { store } = renderPanel({ preferences: { reminders: false } });
    fireEvent.click(switchFor('reminders'));
    expect(prefs(store).reminders).toBe(true);
  });

  it('toggles only the clicked category', () => {
    const { store } = renderPanel();
    fireEvent.click(switchFor('meetings'));
    const after = prefs(store);
    expect(after.meetings).toBe(false);
    for (const id of CATEGORY_IDS.filter(c => c !== 'meetings')) {
      expect(after[id]).toBe(true);
    }
  });

  it('round-trips a category back to its original value', () => {
    const { store } = renderPanel();
    fireEvent.click(switchFor('skills'));
    expect(prefs(store).skills).toBe(false);
    fireEvent.click(switchFor('skills'));
    expect(prefs(store).skills).toBe(true);
  });

  it('re-renders the switch from the new store state after a toggle', () => {
    renderPanel();
    expect(switchFor('important')).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(switchFor('important'));
    expect(switchFor('important')).toHaveAttribute('aria-checked', 'false');
  });

  it.each(CATEGORY_IDS)('dispatches for the "%s" category specifically', id => {
    const { store } = renderPanel();
    fireEvent.click(switchFor(id));
    // Exactly one preference changed, and it is this one.
    const changed = CATEGORY_IDS.filter(c => prefs(store)[c] !== true);
    expect(changed).toEqual([id]);
  });
});

describe('NotificationsPanel — embedded chrome', () => {
  it('renders inside the SettingsPanel chrome by default', () => {
    renderPanel();
    expect(screen.getByTestId('settings-panel-chrome')).toBeInTheDocument();
  });

  it('renders without the chrome when embedded, but keeps the body', () => {
    renderPanel({ embedded: true });
    expect(screen.queryByTestId('settings-panel-chrome')).not.toBeInTheDocument();
    expect(screen.getAllByRole('switch')).toHaveLength(CATEGORY_IDS.length);
  });
});
