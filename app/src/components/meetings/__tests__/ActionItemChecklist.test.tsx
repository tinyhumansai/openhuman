import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { MeetCallActionItem } from '../../../services/meetCallService';
import { renderWithProviders } from '../../../test/test-utils';
import ActionItemChecklist from '../ActionItemChecklist';

const mockNavigate = vi.fn();
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mockNavigate };
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const executableItem: MeetCallActionItem = {
  description: 'Schedule follow-up meeting',
  kind: 'executable',
  tool_name: 'calendar',
  assignee: 'Alice',
};

const advisoryItem: MeetCallActionItem = {
  description: 'Review the proposal',
  kind: 'advisory',
  tool_name: null,
  assignee: 'Bob',
};

describe('ActionItemChecklist', () => {
  it('renders nothing when items list is empty', () => {
    const { container } = renderWithProviders(<ActionItemChecklist items={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it('shows Run with OpenHuman button for executable items', () => {
    renderWithProviders(<ActionItemChecklist items={[executableItem]} />);
    expect(screen.getByText('Run with OpenHuman')).toBeInTheDocument();
  });

  it('does not show Run with OpenHuman button for advisory items', () => {
    renderWithProviders(<ActionItemChecklist items={[advisoryItem]} />);
    expect(screen.queryByText('Run with OpenHuman')).toBeNull();
  });

  it('navigates to /chat when Run with OpenHuman is clicked', () => {
    renderWithProviders(<ActionItemChecklist items={[executableItem]} />);
    fireEvent.click(screen.getByText('Run with OpenHuman'));
    expect(mockNavigate).toHaveBeenCalledWith('/chat');
  });

  it('toggles checkbox on click (cosmetic only)', () => {
    renderWithProviders(<ActionItemChecklist items={[executableItem]} />);
    const checkbox = screen.getByRole('checkbox', { name: executableItem.description });
    expect(checkbox).not.toBeChecked();
    fireEvent.click(checkbox);
    expect(checkbox).toBeChecked();
    fireEvent.click(checkbox);
    expect(checkbox).not.toBeChecked();
  });

  it('renders assignee and tool_name metadata for executable items', () => {
    renderWithProviders(<ActionItemChecklist items={[executableItem]} />);
    expect(screen.getByText(/Alice/)).toBeInTheDocument();
    expect(screen.getByText(/calendar/)).toBeInTheDocument();
  });

  it('renders assignee metadata for advisory items but no tool_name', () => {
    renderWithProviders(<ActionItemChecklist items={[advisoryItem]} />);
    expect(screen.getByText(/Bob/)).toBeInTheDocument();
    // Advisory items don't show tool_name in metadata even if present
    expect(screen.queryByText('Run with OpenHuman')).toBeNull();
  });

  it('renders description text', () => {
    renderWithProviders(<ActionItemChecklist items={[executableItem, advisoryItem]} />);
    expect(screen.getByText(executableItem.description)).toBeInTheDocument();
    expect(screen.getByText(advisoryItem.description)).toBeInTheDocument();
  });
});
