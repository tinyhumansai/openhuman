import { fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import SettingsPanel from './SettingsPanel';

// Pass-through translator so assertions can target the i18n keys directly.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

const navigateBack = vi.fn();
const navigateToSettings = vi.fn();

// Default the active route to a real registry entry ('privacy' → title key
// `pages.settings.account.privacy`) so the auto-derived title is deterministic.
let mockCurrentRoute = 'privacy';
vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    currentRoute: mockCurrentRoute,
    navigateBack,
    navigateToSettings,
    breadcrumbs: [],
  }),
}));

describe('<SettingsPanel />', () => {
  it('derives the panel title from the route registry', () => {
    mockCurrentRoute = 'privacy';
    const { getByRole } = renderWithProviders(
      <SettingsPanel>
        <p>body</p>
      </SettingsPanel>
    );
    // The registry title key for the `privacy` route, rendered as the h2.
    expect(getByRole('heading', { level: 2 })).toHaveTextContent('pages.settings.account.privacy');
  });

  it('lets an explicit title override the registry default', () => {
    mockCurrentRoute = 'privacy';
    const { getByRole } = renderWithProviders(
      <SettingsPanel title="Custom title">
        <p>body</p>
      </SettingsPanel>
    );
    expect(getByRole('heading', { level: 2 })).toHaveTextContent('Custom title');
  });

  it('renders the description and the header action', () => {
    const { getByText } = renderWithProviders(
      <SettingsPanel description="A description" action={<button type="button">Do thing</button>}>
        <p>body</p>
      </SettingsPanel>
    );
    expect(getByText('A description')).toBeInTheDocument();
    expect(getByText('Do thing')).toBeInTheDocument();
  });

  it('renders single-body children', () => {
    const { getByText } = renderWithProviders(
      <SettingsPanel>
        <p>hello body</p>
      </SettingsPanel>
    );
    expect(getByText('hello body')).toBeInTheDocument();
  });

  it('renders chip tabs and swaps to the active tab content', () => {
    const onChange = vi.fn();
    const { getByText, queryByText, getByTestId } = renderWithProviders(
      <SettingsPanel
        value="one"
        onChange={onChange}
        tabsTestIdPrefix="demo-tab"
        tabs={[
          { id: 'one', label: 'One', content: <p>first body</p> },
          { id: 'two', label: 'Two', content: <p>second body</p> },
        ]}
      />
    );
    // Active tab content shows; inactive does not.
    expect(getByText('first body')).toBeInTheDocument();
    expect(queryByText('second body')).not.toBeInTheDocument();

    fireEvent.click(getByTestId('demo-tab-two'));
    expect(onChange).toHaveBeenCalledWith('two');
  });
});
