import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import CanvasTracker from '../CanvasTracker';

vi.mock('../../lib/canvasTracker/hooks', () => ({ useCanvasTracker: vi.fn() }));

const useCanvasTracker = vi.mocked(await import('../../lib/canvasTracker/hooks')).useCanvasTracker;

describe('CanvasTracker', () => {
  beforeEach(() => {
    useCanvasTracker.mockReset();
  });

  it('renders allowlisted courses without showing a token', () => {
    useCanvasTracker.mockReturnValue({
      settings: {
        enabled: true,
        host: 'https://mango-cmu.instructure.com',
        token_set: true,
        allowlisted_courses: [
          { name: '361100-Secrets of the Soil-Lec.001 | 801[3/68]' },
          { name: '515101-Radiation in Everyday Life-Lec.002[3/68]' },
        ],
      },
      tasks: [],
      loading: false,
      syncing: false,
      lastSync: null,
      error: null,
      refresh: vi.fn(),
      syncNow: vi.fn(),
      updateStatus: vi.fn(),
    });

    render(<CanvasTracker />);

    expect(screen.getByText('Canvas Tracker')).toBeInTheDocument();
    expect(screen.getByText(/Secrets of the Soil/)).toBeInTheDocument();
    expect(screen.getByText(/Radiation in Everyday Life/)).toBeInTheDocument();
    expect(screen.queryByText('canvas-secret-token-123')).not.toBeInTheDocument();
  });

  it('updates local status without submitting to Canvas', async () => {
    const updateStatus = vi.fn().mockResolvedValue(undefined);
    useCanvasTracker.mockReturnValue({
      settings: null,
      tasks: [
        {
          course_id: '101',
          course_name: '361100-Secrets of the Soil-Lec.001 | 801[3/68]',
          assignment_id: '55',
          assignment_name: 'Soil reflection',
          due_at: '2026-05-18T06:00:00Z',
          due_at_unclear: false,
          instructions_summary: 'Submit a PDF.',
          submission_type: 'online_upload',
          canvas_workflow_state: 'published',
          canvas_submission_state: null,
          local_status: 'not_started',
          urgency_level: 'high',
          recommended_start_at: '2026-05-16T06:00:00Z',
          reminders_needed: [],
          source_url: null,
          last_seen_at: '2026-05-16T06:00:00Z',
        },
      ],
      loading: false,
      syncing: false,
      lastSync: null,
      error: null,
      refresh: vi.fn(),
      syncNow: vi.fn(),
      updateStatus,
    });

    render(<CanvasTracker />);
    fireEvent.change(screen.getByLabelText('Status for Soil reflection'), {
      target: { value: 'in_progress' },
    });

    await waitFor(() =>
      expect(updateStatus).toHaveBeenCalledWith(expect.any(Object), 'in_progress')
    );
  });
});
