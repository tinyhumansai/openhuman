import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import Intelligence from '../Intelligence';

vi.mock('../../components/intelligence/MemoryWorkspace', () => ({
  MemoryWorkspace: () => <div>Memory workspace</div>,
}));

vi.mock('../../components/intelligence/IntelligenceSubconsciousTab', () => ({
  default: () => <div>Subconscious tab</div>,
}));

vi.mock('../../components/intelligence/IntelligenceTasksTab', () => ({
  default: () => <div>Tasks tab</div>,
}));

vi.mock('../../components/intelligence/IntelligenceCallsTab', () => ({
  default: () => <div>Calls tab</div>,
}));

vi.mock('../../components/intelligence/IntelligenceDreamsTab', () => ({
  default: () => <div>Dreams tab</div>,
}));

vi.mock('../../hooks/useConsciousItems', () => ({
  useConsciousItems: () => ({ isRunning: false }),
}));

vi.mock('../../hooks/useIntelligenceStats', () => ({
  useIntelligenceStats: () => ({ aiStatus: 'ready' }),
}));

vi.mock('../../hooks/useMemoryIngestionStatus', () => ({
  useMemoryIngestionStatus: () => ({ status: { running: false, queueDepth: 0 } }),
}));

const connectMock = vi.fn();

vi.mock('../../hooks/useIntelligenceSocket', () => ({
  useIntelligenceSocket: () => ({ isConnected: true }),
  useIntelligenceSocketManager: () => ({ connect: connectMock }),
}));

vi.mock('../../hooks/useSubconscious', () => ({
  useSubconscious: () => ({
    tasks: [],
    escalations: [],
    logEntries: [],
    status: 'idle',
    loading: false,
    triggering: false,
    triggerTick: vi.fn(),
    addTask: vi.fn(),
    removeTask: vi.fn(),
    toggleTask: vi.fn(),
    approveEscalation: vi.fn(),
    dismissEscalation: vi.fn(),
  }),
}));

describe('Intelligence diagram tab', () => {
  it('shows an architecture diagram viewer from the Intelligence tabs', () => {
    renderWithProviders(<Intelligence />);

    fireEvent.click(screen.getByRole('tab', { name: 'Diagram' }));

    expect(screen.getByRole('heading', { name: 'Architecture Diagram' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh diagram' })).toBeInTheDocument();
  });
});
