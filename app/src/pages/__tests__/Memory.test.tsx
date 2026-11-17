import { fireEvent, screen, waitFor } from '@testing-library/react';
import { useLocation } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { EngineState } from '../../services/api/memoryApi';
import { renderWithProviders } from '../../test/test-utils';
import Memory from '../Memory';

const hoisted = vi.hoisted(() => ({ engineGet: vi.fn(), enginesList: vi.fn() }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryEngineGet: (...a: unknown[]) => hoisted.engineGet(...a),
  memoryEnginesList: (...a: unknown[]) => hoisted.enginesList(...a),
}));

// The tabs have their own suites; here they only need to say which one rendered.
vi.mock('../../components/memory/MemoryEngineTab', () => ({
  default: () => <div data-testid="stub-engine" />,
}));
vi.mock('../../components/memory/MemoryAskTab', () => ({
  default: ({ fetchModes }: { fetchModes: string[] }) => (
    <div data-testid="stub-ask">{fetchModes.join(',')}</div>
  ),
}));
vi.mock('../../components/memory/MemoryExplorerTab', () => ({
  default: () => <div data-testid="stub-explorer" />,
}));
vi.mock('../../components/memory/MemoryLearningsTab', () => ({
  default: () => <div data-testid="stub-learnings" />,
}));
vi.mock('../../components/memory/MemoryConversationsTab', () => ({
  default: () => <div data-testid="stub-conversations" />,
}));
vi.mock('../../components/memory/MemoryBrainTab', () => ({
  default: () => <div data-testid="stub-brain" />,
}));
vi.mock('../../components/memory/MemoryBackgroundTab', () => ({
  default: () => <div data-testid="stub-background" />,
}));
vi.mock('../../components/memory/MemorySettingsTab', () => ({
  default: () => <div data-testid="stub-settings" />,
}));
vi.mock('../../components/memory/MemoryImportBanner', () => ({
  default: ({ engineLabel }: { engineLabel: string }) => (
    <div data-testid="stub-import">{engineLabel}</div>
  ),
}));

const ON: EngineState = {
  engine: 'tinyhumans',
  has_key: false,
  status: 'ok',
  fetch_modes: ['hybrid'],
};
const OFF: EngineState = {
  engine: null,
  has_key: false,
  status: 'off',
  reason: 'Sign in or add a CortexDB key',
  fetch_modes: [],
};

function Where() {
  const { search } = useLocation();
  return <div data-testid="where">{search}</div>;
}

function renderAt(search: string) {
  return renderWithProviders(
    <>
      <Memory />
      <Where />
    </>,
    { initialEntries: [`/connections${search}`] }
  );
}

beforeEach(() => {
  hoisted.engineGet.mockReset().mockResolvedValue(ON);
  hoisted.enginesList
    .mockReset()
    .mockResolvedValue({
      engines: [{ id: 'tinyhumans', label: 'TinyHumans' }],
      active: 'tinyhumans',
    });
});

describe('Memory page', () => {
  it('renders the nine chips', async () => {
    renderAt('?tab=brain');
    for (const chip of [
      'engine',
      'migration',
      'ask',
      'explorer',
      'learnings',
      'conversations',
      'brain',
      'background',
      'settings',
    ]) {
      expect(await screen.findByTestId(`brain-tab-${chip}`)).toBeInTheDocument();
    }
  });

  it('defaults to Engine when an engine is active', async () => {
    renderAt('?tab=brain');
    expect(await screen.findByTestId('stub-engine')).toBeInTheDocument();
    expect(screen.queryByTestId('stub-ask')).not.toBeInTheDocument();
    expect(screen.queryByTestId('stub-import')).not.toBeInTheDocument();
  });

  it('defaults to Engine when memory is off', async () => {
    hoisted.engineGet.mockResolvedValue(OFF);
    renderAt('?tab=brain');
    expect(await screen.findByTestId('stub-engine')).toBeInTheDocument();
    expect(screen.queryByTestId('stub-import')).not.toBeInTheDocument();
  });

  it.each(['engine', 'migration', 'ask', 'settings'])(
    'shows the alpha notice on the %s chip',
    async chip => {
      renderAt(`?tab=brain&brain=${chip}`);
      expect(await screen.findByTestId('memory-alpha-notice')).toHaveTextContent(
        'Early Alpha: Memory is still being tested.'
      );
    }
  );

  it('shows the alpha notice while memory is off', async () => {
    hoisted.engineGet.mockResolvedValue(OFF);
    renderAt('?tab=brain&brain=ask');
    expect(await screen.findByTestId('memory-off-state')).toBeInTheDocument();
    expect(screen.getByTestId('memory-alpha-notice')).toBeInTheDocument();
  });

  it('keeps the import flow on the Migration chip only', async () => {
    renderAt('?tab=brain&brain=migration');
    expect(await screen.findByTestId('stub-import')).toHaveTextContent('TinyHumans');
    expect(screen.queryByTestId('memory-cortex-announcement')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('brain-tab-ask'));
    expect(await screen.findByTestId('stub-ask')).toBeInTheDocument();
    expect(screen.queryByTestId('stub-import')).not.toBeInTheDocument();
  });

  it('shows the off state on Migration while memory is off', async () => {
    hoisted.engineGet.mockResolvedValue(OFF);
    renderAt('?tab=brain&brain=migration');
    expect(await screen.findByTestId('memory-off-state')).toBeInTheDocument();
    expect(screen.queryByTestId('stub-import')).not.toBeInTheDocument();
  });

  it.each([
    ['explorer', 'stub-explorer'],
    ['learnings', 'stub-learnings'],
    ['conversations', 'stub-conversations'],
    ['brain', 'stub-brain'],
    ['background', 'stub-background'],
    ['settings', 'stub-settings'],
    ['engine', 'stub-engine'],
  ])('opens the %s chip from ?brain=', async (chip, testId) => {
    renderAt(`?tab=brain&brain=${chip}`);
    expect(await screen.findByTestId(testId)).toBeInTheDocument();
  });

  it.each([
    ['graph', 'ask', 'stub-ask'],
    ['goals', 'ask', 'stub-ask'],
    ['sources', 'brain', 'stub-brain'],
    ['sync', 'brain', 'stub-brain'],
    ['history', 'brain', 'stub-brain'],
    ['documents', 'brain', 'stub-brain'],
    ['context', 'ask', 'stub-ask'],
  ])('rewrites legacy ?brain=%s to %s', async (legacy, chip, testId) => {
    renderAt(`?tab=brain&brain=${legacy}&view=history`);
    expect(await screen.findByTestId(testId)).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId('where')).toHaveTextContent(`?tab=brain&brain=${chip}`)
    );
    expect(screen.getByTestId('where').textContent).not.toContain('view=');
  });

  it('rewrites an unknown ?brain= value to the Engine chip', async () => {
    renderAt('?tab=brain&brain=bogus');
    expect(await screen.findByTestId('stub-engine')).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId('where')).toHaveTextContent('?tab=brain&brain=engine')
    );
    expect(screen.getByTestId('where').textContent).not.toContain('bogus');
  });

  it('switches chips through the URL', async () => {
    renderAt('?tab=brain&brain=ask');
    await screen.findByTestId('stub-ask');
    fireEvent.click(screen.getByTestId('brain-tab-learnings'));
    expect(await screen.findByTestId('stub-learnings')).toBeInTheDocument();
    expect(screen.getByTestId('where')).toHaveTextContent('brain=learnings');
  });

  it('shows the off state on non-engine chips and links to the Engine chip', async () => {
    hoisted.engineGet.mockResolvedValue(OFF);
    renderAt('?tab=brain&brain=brain');
    expect(await screen.findByTestId('memory-off-state')).toHaveTextContent(
      'Sign in or add a CortexDB key'
    );
    expect(screen.queryByTestId('stub-brain')).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId('memory-off-open-engine'));
    expect(await screen.findByTestId('stub-engine')).toBeInTheDocument();
    expect(screen.getByTestId('where')).toHaveTextContent('brain=engine');
  });

  it('treats an unreadable engine as off, says why, and retries', async () => {
    hoisted.engineGet.mockRejectedValueOnce(new Error('core unreachable'));
    renderAt('?tab=brain&brain=ask');
    expect(await screen.findByTestId('memory-load-error')).toHaveTextContent('core unreachable');
    expect(screen.getByTestId('memory-off-state')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('memory-load-retry'));
    expect(await screen.findByTestId('stub-ask')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-load-error')).not.toBeInTheDocument();
  });
});
