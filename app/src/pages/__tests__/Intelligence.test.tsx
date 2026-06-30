import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Stub the heavy tab content + chrome so the test exercises only the
// URL-backed tab selection logic in Intelligence.tsx.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
vi.mock('../../components/intelligence/MemorySection', () => ({
  default: () => <div data-testid="tab-memory" />,
}));
vi.mock('../../components/intelligence/IntelligenceSubconsciousTab', () => ({
  default: () => <div data-testid="tab-subconscious" />,
}));
vi.mock('../../components/intelligence/IntelligenceTasksTab', () => ({
  default: () => <div data-testid="tab-tasks" />,
}));
vi.mock('../../components/intelligence/ModelCouncilTab', () => ({
  default: () => <div data-testid="tab-council" />,
}));
vi.mock('../../components/intelligence/WorkflowsTab', () => ({
  default: () => <div data-testid="tab-workflows" />,
}));
vi.mock('../../components/intelligence/Toast', () => ({ ToastContainer: () => null }));
vi.mock('../../components/intelligence/ConfirmationModal', () => ({
  ConfirmationModal: () => null,
}));
interface MockChipTabsProps {
  value: string;
  onChange: (tab: string) => void;
}
vi.mock('../../components/layout/ChipTabs', () => ({
  default: ({ value, onChange }: MockChipTabsProps) => (
    <div data-testid="pilltabs">
      <span>selected:{value}</span>
      {['memory', 'subconscious', 'tasks', 'workflows', 'council'].map(tab => (
        <button key={tab} type="button" onClick={() => onChange(tab)}>
          go-{tab}
        </button>
      ))}
    </div>
  ),
}));
vi.mock('../../hooks/useIntelligenceSocket', () => ({
  useIntelligenceSocket: () => ({ isConnected: true }),
  useIntelligenceSocketManager: () => ({}),
}));
// useDeveloperMode needs a Redux Provider; mock it so tests render without one.
// All tabs are visible (developer mode on) — the test exercises URL routing,
// not the dev gate, so a stable open-gate is fine here.
vi.mock('../../hooks/useDeveloperMode', () => ({ useDeveloperMode: () => true }));
vi.mock('../../hooks/useSubconscious', () => ({
  useSubconscious: () => ({
    status: 'idle',
    mode: 'manual',
    intervalMinutes: 30,
    triggering: false,
    settingMode: false,
    triggerTick: vi.fn(),
    setMode: vi.fn(),
    setIntervalMinutes: vi.fn(),
  }),
}));

const Intelligence = (await import('../Intelligence')).default;

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Intelligence />
    </MemoryRouter>
  );
}

describe('Intelligence URL-backed tab', () => {
  beforeEach(() => vi.clearAllMocks());

  it('defaults to the tasks tab when no ?tab is present', () => {
    renderAt('/intelligence');
    expect(screen.getByTestId('tab-tasks')).toBeInTheDocument();
    expect(screen.getByText('selected:tasks')).toBeInTheDocument();
  });

  it('honours ?tab=tasks from the URL', () => {
    renderAt('/intelligence?tab=tasks');
    expect(screen.getByTestId('tab-tasks')).toBeInTheDocument();
    expect(screen.getByText('selected:tasks')).toBeInTheDocument();
  });

  it('falls back to tasks for an unknown ?tab value', () => {
    renderAt('/intelligence?tab=bogus');
    expect(screen.getByTestId('tab-tasks')).toBeInTheDocument();
  });

  it('switching tabs updates the active tab via the URL', () => {
    renderAt('/intelligence');
    fireEvent.click(screen.getByText('go-council'));
    expect(screen.getByTestId('tab-council')).toBeInTheDocument();
    expect(screen.getByText('selected:council')).toBeInTheDocument();
  });
});
