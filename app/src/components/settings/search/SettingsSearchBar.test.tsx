/**
 * Tests for SettingsSearchBar — the global Settings search input + result list.
 *
 * The translator is mocked to identity so we can assert against the stable i18n
 * keys, and the registry's own keys (e.g. 'settings.appearance.title') contain
 * the words we search for. Navigation and developer-mode are mocked so the bar
 * is exercised in isolation.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import SettingsSearchBar from './SettingsSearchBar';

const hoisted = vi.hoisted(() => ({ devMode: false, navigate: vi.fn() }));

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
vi.mock('../../../hooks/useDeveloperMode', () => ({ useDeveloperMode: () => hoisted.devMode }));
vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateToSettings: hoisted.navigate }),
}));

// Controlled wrapper so typing flows through value/onValueChange like in
// SettingsHome.
const Harness = () => {
  const [value, setValue] = useState('');
  return <SettingsSearchBar value={value} onValueChange={setValue} />;
};

const type = (text: string) =>
  fireEvent.change(screen.getByTestId('settings-search-input'), { target: { value: text } });

beforeEach(() => {
  hoisted.devMode = false;
  hoisted.navigate.mockReset();
});

describe('SettingsSearchBar', () => {
  test('shows no results list until a query is entered', () => {
    render(<Harness />);
    expect(screen.queryByTestId('settings-search-results')).toBeNull();
    expect(screen.queryByTestId('settings-search-empty')).toBeNull();
  });

  test('filters entries by query and navigates on click', () => {
    render(<Harness />);
    type('appearance');

    const result = screen.getByTestId('settings-search-result-appearance');
    expect(result).toBeTruthy();

    fireEvent.click(result);
    expect(hoisted.navigate).toHaveBeenCalledWith('appearance');
  });

  test('renders an empty state when nothing matches', () => {
    render(<Harness />);
    type('zzzznomatchqq');
    expect(screen.getByTestId('settings-search-empty')).toBeTruthy();
    expect(screen.queryByTestId('settings-search-results')).toBeNull();
  });

  test('hides developer-only destinations unless developer mode is on', () => {
    render(<Harness />);
    // 'cron-jobs' is a devOnly entry.
    type('cron');
    expect(screen.queryByTestId('settings-search-result-cron-jobs')).toBeNull();
  });

  test('surfaces developer-only destinations when developer mode is on', () => {
    hoisted.devMode = true;
    render(<Harness />);
    type('cron');
    expect(screen.getByTestId('settings-search-result-cron-jobs')).toBeTruthy();
  });

  test('Enter activates the highlighted result', () => {
    render(<Harness />);
    const input = screen.getByTestId('settings-search-input');
    type('appearance');
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(hoisted.navigate).toHaveBeenCalledWith('appearance');
  });

  test('Escape clears the query', () => {
    render(<Harness />);
    const input = screen.getByTestId('settings-search-input') as HTMLInputElement;
    type('appearance');
    expect(input.value).toBe('appearance');
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(input.value).toBe('');
  });

  test('clear button empties the query', () => {
    render(<Harness />);
    const input = screen.getByTestId('settings-search-input') as HTMLInputElement;
    type('appearance');
    fireEvent.click(screen.getByTestId('settings-search-clear'));
    expect(input.value).toBe('');
  });
});
