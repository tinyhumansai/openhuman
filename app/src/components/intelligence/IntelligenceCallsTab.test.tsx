import { render, screen } from '@testing-library/react';
// import { fireEvent, waitFor } from '@testing-library/react'; // re-enable with the full UI
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// import { closeMeetCall, joinMeetCall } from '../../services/meetCallService'; // re-enable with the full UI
import IntelligenceCallsTab from './IntelligenceCallsTab';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

vi.mock('../../services/meetCallService', () => ({
  joinMeetCall: vi.fn(),
  closeMeetCall: vi.fn(),
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
});
