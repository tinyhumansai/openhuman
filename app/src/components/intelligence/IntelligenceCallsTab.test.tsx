import { render, screen } from '@testing-library/react';
// import { fireEvent, waitFor } from '@testing-library/react'; // re-enable with the full UI
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { joinMeetCall } from '../../services/meetCallService';
import IntelligenceCallsTab from './IntelligenceCallsTab';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

vi.mock('../../services/meetCallService', () => ({
  joinMeetCall: vi.fn(),
  closeMeetCall: vi.fn(),
  listMeetCalls: vi.fn().mockResolvedValue([]),
}));

describe('IntelligenceCallsTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders coming soon placeholder', () => {
    render(<IntelligenceCallsTab />);
    expect(screen.getByText('Calls')).toBeInTheDocument();
    expect(screen.getByText('Coming Soon')).toBeInTheDocument();
  });

  it('does not render a form in the coming soon view', () => {
    render(<IntelligenceCallsTab />);
    expect(screen.queryByRole('form')).not.toBeInTheDocument();
  });

  it('accepts an onToast prop without throwing', () => {
    const onToast = vi.fn();
    expect(() => render(<IntelligenceCallsTab onToast={onToast} />)).not.toThrow();
  });

  it('does not call joinMeetCall on initial render', () => {
    render(<IntelligenceCallsTab />);
    expect(joinMeetCall).not.toHaveBeenCalled();
  });
});
