import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import Intelligence from './Intelligence';

vi.mock('../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

// IS_DEV / useDeveloperMode gate the dev-only "council" tab; default to
// a non-dev build so the gate is closed unless the test overrides it.
const isDev = vi.hoisted(() => ({ value: false }));
vi.mock('../utils/config', async () => {
  const actual = await vi.importActual<typeof import('../utils/config')>('../utils/config');
  return {
    ...actual,
    get IS_DEV() {
      return isDev.value;
    },
  };
});
// useDeveloperMode combines IS_DEV with the persisted Redux preference.
// Mock it here so tests don't need a Redux Provider — just respect isDev.value.
vi.mock('../hooks/useDeveloperMode', () => ({ useDeveloperMode: () => isDev.value }));

// Heavy hooks → minimal stubs.
vi.mock('../hooks/useIntelligenceSocket', () => ({
  useIntelligenceSocket: () => ({ isConnected: true }),
  useIntelligenceSocketManager: () => ({ connect: vi.fn() }),
}));
vi.mock('../hooks/useSubconscious', () => ({
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

// Tab content → identifiable stubs so we can assert which tab is active.
vi.mock('../components/intelligence/MemorySection', () => ({
  default: () => <div>MEMORY_PANEL</div>,
}));
vi.mock('../components/intelligence/IntelligenceSubconsciousTab', () => ({
  default: () => <div>SUBCON_PANEL</div>,
}));
vi.mock('../components/intelligence/IntelligenceTasksTab', () => ({
  default: () => <div>TASKS_PANEL</div>,
}));
vi.mock('../components/intelligence/WorkflowsTab', () => ({
  default: () => <div>WORKFLOWS_PANEL</div>,
}));
vi.mock('../components/intelligence/ModelCouncilTab', () => ({
  default: () => <div>COUNCIL_PANEL</div>,
}));

const renderAt = (path: string) =>
  render(
    <MemoryRouter initialEntries={[path]}>
      <Intelligence />
    </MemoryRouter>
  );

describe('Intelligence tab selection', () => {
  beforeEach(() => {
    isDev.value = false;
  });

  it('defaults to the Tasks tab with no ?tab=', async () => {
    renderAt('/intelligence');
    await waitFor(() => expect(screen.getByText('TASKS_PANEL')).toBeInTheDocument());
  });

  it('honours ?tab=workflows deep link', async () => {
    renderAt('/intelligence?tab=workflows');
    await waitFor(() => expect(screen.getByText('WORKFLOWS_PANEL')).toBeInTheDocument());
  });

  it('ignores a dev-only ?tab=council in non-dev builds (falls back to tasks)', async () => {
    isDev.value = false;
    renderAt('/intelligence?tab=council');
    await waitFor(() => expect(screen.getByText('TASKS_PANEL')).toBeInTheDocument());
    expect(screen.queryByText('COUNCIL_PANEL')).not.toBeInTheDocument();
  });

  it('honours ?tab=council when IS_DEV', async () => {
    isDev.value = true;
    renderAt('/intelligence?tab=council');
    await waitFor(() => expect(screen.getByText('COUNCIL_PANEL')).toBeInTheDocument());
  });

  it('switches panel when a tab pill is clicked', async () => {
    renderAt('/intelligence?tab=tasks');
    await waitFor(() => expect(screen.getByText('TASKS_PANEL')).toBeInTheDocument());
    // ChipTabs renders the tab labels (i18n keys); click the Workflows pill.
    fireEvent.click(screen.getByText('memory.tab.workflows'));
    await waitFor(() => expect(screen.getByText('WORKFLOWS_PANEL')).toBeInTheDocument());
  });
});
