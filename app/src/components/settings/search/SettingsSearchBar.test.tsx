/**
 * Tests for SettingsSearchBar — the settings sidebar search input.
 *
 * The two-pane restructure reduced this to a plain controlled text input: it no
 * longer renders its own result list or performs navigation. The parent
 * (SettingsSidebar) consumes the query to filter the visible nav tabs in place,
 * so these tests cover only the input's own behavior (typing, Escape, clear).
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, test, vi } from 'vitest';

import SettingsSearchBar from './SettingsSearchBar';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

// Controlled wrapper so typing flows through value/onValueChange like in
// SettingsSidebar.
const Harness = () => {
  const [value, setValue] = useState('');
  return <SettingsSearchBar value={value} onValueChange={setValue} />;
};

const type = (text: string) =>
  fireEvent.change(screen.getByTestId('settings-search-input'), { target: { value: text } });

describe('SettingsSearchBar', () => {
  test('renders the input without any embedded result list', () => {
    render(<Harness />);
    expect(screen.getByTestId('settings-search-input')).toBeTruthy();
    // The bar itself never renders a result dropdown — filtering lives in the
    // sidebar.
    expect(screen.queryByTestId('settings-search-results')).toBeNull();
    expect(screen.queryByTestId('settings-search-empty')).toBeNull();
  });

  test('reflects typed text in the controlled value', () => {
    render(<Harness />);
    const input = screen.getByTestId('settings-search-input') as HTMLInputElement;
    type('appearance');
    expect(input.value).toBe('appearance');
  });

  test('shows the clear button only once a query is entered', () => {
    render(<Harness />);
    expect(screen.queryByTestId('settings-search-clear')).toBeNull();
    type('appearance');
    expect(screen.getByTestId('settings-search-clear')).toBeTruthy();
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
