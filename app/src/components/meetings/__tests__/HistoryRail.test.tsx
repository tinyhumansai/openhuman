import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { MeetCallRecord } from '../../../services/meetCallService';
import { renderWithProviders } from '../../../test/test-utils';
import HistoryRail, { type CallGroup } from '../HistoryRail';

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const call1: MeetCallRecord = {
  request_id: 'req-1',
  meet_url: 'https://meet.google.com/abc-def-ghi',
  bot_display_name: 'OpenHuman',
  owner_display_name: 'Alice',
  started_at_ms: Date.now() - 3600000,
  ended_at_ms: Date.now() - 3000000,
  listened_seconds: 300,
  spoken_seconds: 60,
  turn_count: 5,
  participants: ['Alice', 'Bob'],
};

const call2: MeetCallRecord = {
  request_id: 'req-2',
  meet_url: 'https://zoom.us/j/123456',
  bot_display_name: 'OpenHuman',
  owner_display_name: 'Carol',
  started_at_ms: Date.now() - 86400000 - 3600000,
  ended_at_ms: Date.now() - 86400000 - 3000000,
  listened_seconds: 120,
  spoken_seconds: 30,
  turn_count: 2,
  participants: ['Carol'],
};

const groups: CallGroup[] = [
  { label: 'Today', calls: [call1] },
  { label: 'Yesterday', calls: [call2] },
];

function renderRail(overrides?: Partial<Parameters<typeof HistoryRail>[0]>) {
  const props = {
    groups,
    selectedId: null,
    onSelect: vi.fn(),
    searchQuery: '',
    onSearchChange: vi.fn(),
    platformFilter: '',
    onPlatformChange: vi.fn(),
    ...overrides,
  };
  return { ...renderWithProviders(<HistoryRail {...props} />), props };
}

describe('HistoryRail', () => {
  it('renders group labels', () => {
    renderRail();
    expect(screen.getByText('Today')).toBeInTheDocument();
    expect(screen.getByText('Yesterday')).toBeInTheDocument();
  });

  it('renders meeting codes for each call', () => {
    renderRail();
    expect(screen.getByText('abc-def-ghi')).toBeInTheDocument();
    expect(screen.getByText('j/123456')).toBeInTheDocument();
  });

  it('renders platform logo for google meet', () => {
    renderRail();
    const imgs = screen.getAllByRole('img');
    const gmeetImg = imgs.find(img => (img as HTMLImageElement).alt === 'Google Meet');
    expect(gmeetImg).toBeDefined();
  });

  it('renders turn count for each call', () => {
    renderRail();
    // turn_count=5 should show plural "5 turns"
    expect(screen.getByText(/5 turn/)).toBeInTheDocument();
  });

  it('fires onSelect when a row is clicked', () => {
    const onSelect = vi.fn();
    renderRail({ onSelect });
    // The first button is now the platform-filter toggle; pick the call row by
    // its meeting code instead.
    const rowButton = screen
      .getAllByRole('button')
      .find(b => b.textContent?.includes('abc-def-ghi'));
    expect(rowButton).toBeDefined();
    fireEvent.click(rowButton!);
    expect(onSelect).toHaveBeenCalledWith('req-1');
  });

  it('highlights the selected row', () => {
    const { container } = renderRail({ selectedId: 'req-1' });
    const selectedBtn = container.querySelector('button.bg-primary-50');
    expect(selectedBtn).not.toBeNull();
  });

  it('reflects search query in the input', () => {
    renderRail({ searchQuery: 'abc' });
    const input = screen.getByRole('searchbox') as HTMLInputElement;
    expect(input.value).toBe('abc');
  });

  it('calls onSearchChange when search input changes', () => {
    const onSearchChange = vi.fn();
    renderRail({ onSearchChange });
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'zoom' } });
    expect(onSearchChange).toHaveBeenCalledWith('zoom');
  });

  it('shows platform options (with names) only after opening the filter menu', () => {
    renderRail();
    // Collapsed: the filter is icon-only — names are not shown yet.
    expect(screen.queryByText('All platforms')).not.toBeInTheDocument();
    // Open the menu via the filter button.
    fireEvent.click(screen.getByRole('button', { name: /all platforms/i }));
    expect(screen.getByText('All platforms')).toBeInTheDocument();
    expect(screen.getByRole('option', { name: /zoom/i })).toBeInTheDocument();
  });

  it('calls onPlatformChange when a platform is picked from the menu', () => {
    const onPlatformChange = vi.fn();
    renderRail({ onPlatformChange });
    fireEvent.click(screen.getByRole('button', { name: /all platforms/i }));
    fireEvent.click(screen.getByRole('option', { name: /zoom/i }));
    expect(onPlatformChange).toHaveBeenCalledWith('zoom');
  });

  it('renders empty state when no groups have calls', () => {
    renderRail({ groups: [{ label: 'Today', calls: [] }] });
    expect(screen.getByText(/no.*call/i)).toBeInTheDocument();
  });
});
