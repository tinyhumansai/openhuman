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
vi.mock('../../components/memory/MemoryLearningsTab', () => ({
  default: () => <div data-testid="stub-learnings" />,
}));
vi.mock('../../components/memory/MemoryConversationsTab', () => ({
  default: () => <div data-testid="stub-conversations" />,
}));
vi.mock('../../components/memory/MemoryDocumentsTab', () => ({
  default: () => <div data-testid="stub-documents" />,
}));
vi.mock('../../components/memory/MemoryContextTab', () => ({
  default: () => <div data-testid="stub-context" />,
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
  it('renders the six chips', async () => {
    renderAt('?tab=brain');
    for (const chip of ['engine', 'ask', 'learnings', 'conversations', 'documents', 'context']) {
      expect(await screen.findByTestId(`brain-tab-${chip}`)).toBeInTheDocument();
    }
  });

  it('defaults to Ask when an engine is active', async () => {
    renderAt('?tab=brain');
    expect(await screen.findByTestId('stub-ask')).toHaveTextContent('hybrid');
    expect(screen.getByTestId('stub-import')).toHaveTextContent('TinyHumans');
  });

  it('defaults to Engine when memory is off', async () => {
    hoisted.engineGet.mockResolvedValue(OFF);
    renderAt('?tab=brain');
    expect(await screen.findByTestId('stub-engine')).toBeInTheDocument();
    expect(screen.queryByTestId('stub-import')).not.toBeInTheDocument();
  });

  it.each([
    ['learnings', 'stub-learnings'],
    ['conversations', 'stub-conversations'],
    ['documents', 'stub-documents'],
    ['context', 'stub-context'],
    ['engine', 'stub-engine'],
  ])('opens the %s chip from ?brain=', async (chip, testId) => {
    renderAt(`?tab=brain&brain=${chip}`);
    expect(await screen.findByTestId(testId)).toBeInTheDocument();
  });

  it.each([
    ['graph', 'ask', 'stub-ask'],
    ['goals', 'ask', 'stub-ask'],
    ['sources', 'documents', 'stub-documents'],
    ['sync', 'documents', 'stub-documents'],
    ['history', 'documents', 'stub-documents'],
  ])('rewrites legacy ?brain=%s to %s', async (legacy, chip, testId) => {
    renderAt(`?tab=brain&brain=${legacy}&view=history`);
    expect(await screen.findByTestId(testId)).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId('where')).toHaveTextContent(`?tab=brain&brain=${chip}`)
    );
    expect(screen.getByTestId('where').textContent).not.toContain('view=');
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
    renderAt('?tab=brain&brain=documents');
    expect(await screen.findByTestId('memory-off-state')).toHaveTextContent(
      'Sign in or add a CortexDB key'
    );
    expect(screen.queryByTestId('stub-documents')).not.toBeInTheDocument();
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
