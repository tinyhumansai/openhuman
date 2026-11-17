import {
  AssistantRuntimeProvider,
  type ExternalThreadQueueAdapter,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { RunQueueItem } from '../../../store/queueSlice';
import { ComposerMessageQueue } from './ComposerMessageQueue';
import { buildOpenHumanQueueAdapter } from './queueAdapter';

const TRANSCRIPT: ThreadMessageLike[] = [
  { role: 'user', content: 'older question' },
  { role: 'assistant', content: 'older answer' },
  { role: 'user', content: 'summarise the launch plan' },
];

function Harness({ queue }: { queue: ExternalThreadQueueAdapter }) {
  const runtime = useExternalStoreRuntime({
    messages: TRANSCRIPT,
    isRunning: true,
    convertMessage: (m: ThreadMessageLike) => m,
    onNew: async () => {},
    queue,
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <ComposerMessageQueue />
    </AssistantRuntimeProvider>
  );
}

function renderWith(items: RunQueueItem[], remove = vi.fn()) {
  const queue = buildOpenHumanQueueAdapter({ items, send: vi.fn(), remove });
  render(<Harness queue={queue} />);
  return { remove };
}

describe('ComposerMessageQueue', () => {
  it('renders nothing while the queue is empty', () => {
    renderWith([]);
    expect(screen.queryByTestId('queued-followups')).not.toBeInTheDocument();
  });

  it('shows the running prompt and each queued message, with translated captions', () => {
    renderWith([
      { id: 'q1', lane: null, textPreview: 'and the pricing?' },
      { id: 'q2', lane: null, textPreview: 'and the timeline' },
    ]);

    const strip = screen.getByTestId('queued-followups');
    expect(strip).toHaveTextContent('summarise the launch plan');
    expect(strip).toHaveTextContent('and the pricing?');
    expect(strip).toHaveTextContent('and the timeline');
    expect(strip).toHaveTextContent('2 queued');
    expect(strip).toHaveTextContent('sends when this finishes');
    expect(strip).toHaveTextContent('running');
  });

  it('removes an item through the runtime queue', () => {
    const { remove } = renderWith([{ id: 'q1', lane: null, textPreview: 'drop me' }]);

    fireEvent.click(screen.getByRole('button', { name: 'Remove "drop me" from the queue' }));

    expect(remove).toHaveBeenCalledWith('q1');
  });

  it('gives a queued row no icon besides the remove button (#7730)', () => {
    renderWith([{ id: 'q1', lane: null, textPreview: 'keep me' }]);

    const row = screen.getByText('keep me').closest('li');
    expect(row).not.toBeNull();
    // Only the remove button's X: no decorative arrow posing as "send now".
    expect(row!.querySelectorAll('svg')).toHaveLength(1);
  });
});
