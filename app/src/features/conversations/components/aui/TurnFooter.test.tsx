import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Thread } from '../../../../components/assistant-ui/thread';
import type { TurnProcessTrail } from '../../../../providers/assistantUiMessages';
import { TurnFooter } from './TurnFooter';
import { TurnFooterHost } from './turnFooterHost';

const trail = (over: Partial<TurnProcessTrail> = {}): TurnProcessTrail => ({
  steps: 8,
  tools: 2,
  timeline: [],
  transcript: [],
  ...over,
});

function mount(
  metadata: Record<string, unknown> | undefined,
  onOpenTurnProcess?: (t: TurnProcessTrail) => void
) {
  const messages: ThreadMessageLike[] = [
    { role: 'user', content: [{ type: 'text', text: 'hello' }] },
    {
      role: 'assistant',
      content: [{ type: 'text', text: 'the answer' }],
      ...(metadata ? { metadata: { custom: metadata } } : {}),
    },
  ];
  function Harness() {
    const runtime = useExternalStoreRuntime({
      messages,
      isRunning: false,
      convertMessage: (m: ThreadMessageLike) => m,
      onNew: async () => {},
    });
    return (
      <AssistantRuntimeProvider runtime={runtime}>
        <TurnFooterHost onOpenTurnProcess={onOpenTurnProcess}>
          <Thread components={{ TurnFooter }} />
        </TurnFooterHost>
      </AssistantRuntimeProvider>
    );
  }
  return render(<Harness />);
}

describe('TurnFooter', () => {
  it('summarises the turn and opens the rail on that turn’s own trail', () => {
    const open = vi.fn();
    mount({ processTrail: trail() }, open);

    const footer = screen.getByTestId('turn-process-footer');
    expect(footer.textContent).toBe('8 steps · 2 tools');

    fireEvent.click(footer);
    // The trail travels with the click: the host needs no second lookup, and
    // cannot show a different turn's trail by mistake.
    expect(open).toHaveBeenCalledWith(expect.objectContaining({ steps: 8, tools: 2 }));
  });

  it('drops the tool clause for a turn that called none', () => {
    mount({ processTrail: trail({ steps: 3, tools: 0 }) }, vi.fn());
    expect(screen.getByTestId('turn-process-footer').textContent).toBe('3 steps');
  });

  it('reads as process, not answer: muted and no larger than 12px', () => {
    // The whole point of moving reasoning and narration off this surface is
    // that what is left reads as one class of thing. jsdom performs no layout,
    // so the utilities are the only observable — but they are the same two the
    // in-flight status line carries (`InferenceStatusLine`), which is the
    // invariant worth pinning: one treatment, not three greys.
    mount({ processTrail: trail() }, vi.fn());
    const classes = screen.getByTestId('turn-process-footer').className.split(/\s+/);
    expect(classes).toContain('text-content-muted');
    expect(classes).toContain('text-xs');
  });

  it('renders no door for a turn with no process behind it', () => {
    mount({ processTrail: null }, vi.fn());
    expect(screen.queryByTestId('turn-process-footer')).toBeNull();
  });

  it('renders nothing when no host is mounted to open the rail', () => {
    mount({ processTrail: trail() }, undefined);
    expect(screen.queryByTestId('turn-process-footer')).toBeNull();
  });

  it('survives a message this projection did not build', () => {
    // The footer can be mounted on the kit's own runtime or a test harness,
    // where `metadata.custom` is absent or shaped differently.
    mount(undefined, vi.fn());
    expect(screen.queryByTestId('turn-process-footer')).toBeNull();
    mount({ processTrail: { steps: 'lots' } }, vi.fn());
    expect(screen.queryByTestId('turn-process-footer')).toBeNull();
  });
});
