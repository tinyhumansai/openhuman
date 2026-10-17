import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { StallWarning } from './StallWarning';

const COPY: Record<string, string> = {
  'chat.status.quietFor': 'Quiet for {duration}.',
  'chat.stallWarning.thinking': 'Still thinking.',
  'chat.stallWarning.working': 'No new output for {duration}. The agent may be stuck.',
  'chat.stallWarning.toolRunning': '{tool}. Long steps can take a while.',
  'conversations.tools.useApp.active': 'Using {app}',
};

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string, fallback?: string) => COPY[key] ?? fallback ?? key }),
}));

function shellCall(status: ToolTimelineEntry['status'] = 'running'): ToolTimelineEntry {
  return {
    id: 'call-1',
    name: 'shell',
    round: 1,
    seq: 1,
    status,
    argsBuffer: '{"command":"gh run watch 123"}',
  };
}

describe('StallWarning', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 9, 7, 12, 0, 0));
  });
  afterEach(() => vi.useRealTimers());

  it('names a running tool and how long the turn has been quiet in a neutral status', () => {
    const onStop = vi.fn();
    render(
      <StallWarning
        phase="tool_use"
        quietSince={Date.now() - 121_000}
        runningTool={shellCall()}
        onStop={onStop}
      />
    );

    const warning = screen.getByTestId('chat-stall-warning');
    expect(warning).toHaveAttribute('data-chat-stall-state', 'tool');
    expect(warning).toHaveTextContent('Running command. Long steps can take a while.');
    expect(screen.getByTestId('chat-stall-quiet')).toHaveTextContent('Quiet for 2m 1s.');
    expect(warning).not.toHaveTextContent('stuck');
    expect(warning.closest('[data-slot="alert"]')).toHaveAttribute('data-variant', 'default');

    act(() => {
      vi.advanceTimersByTime(5_000);
    });
    expect(screen.getByTestId('chat-stall-quiet')).toHaveTextContent('Quiet for 2m 6s.');

    const stop = screen.getByTestId('chat-stall-stop');
    expect(stop).toHaveAttribute('data-variant', 'tertiary');
    fireEvent.click(stop);
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it('names the app a delegated integrations agent is using, as the timeline does', () => {
    render(
      <StallWarning
        phase="tool_use"
        quietSince={Date.now() - 120_000}
        runningTool={{
          id: 'call-2',
          name: 'subagent:integrations_agent',
          round: 1,
          seq: 2,
          status: 'running',
          sourceToolName: 'delegate_notion',
        }}
        onStop={vi.fn()}
      />
    );

    expect(screen.getByTestId('chat-stall-warning')).toHaveTextContent('Using Notion.');
  });

  it('warns in amber when nothing is running past the window', () => {
    render(<StallWarning phase="tool_use" quietSince={Date.now() - 120_000} onStop={vi.fn()} />);

    const warning = screen.getByTestId('chat-stall-warning');
    expect(warning).toHaveAttribute('data-chat-stall-state', 'stuck');
    expect(warning).toHaveTextContent('No new output for 2m 0s. The agent may be stuck.');
    expect(warning.closest('[data-slot="alert"]')).toHaveAttribute('data-variant', 'warning');
    expect(screen.getByTestId('chat-stall-stop')).toHaveAttribute('data-variant', 'secondary');
  });

  it('shows the calm thinking copy with a ticking quiet duration', () => {
    render(<StallWarning phase="thinking" quietSince={Date.now() - 120_000} onStop={vi.fn()} />);

    const warning = screen.getByTestId('chat-stall-warning');
    expect(warning).toHaveAttribute('data-chat-stall-phase', 'thinking');
    expect(warning).toHaveAttribute('data-chat-stall-state', 'thinking');
    expect(warning).toHaveTextContent('Still thinking.');
    expect(warning.closest('[data-slot="alert"]')).toHaveAttribute('data-variant', 'default');
    expect(screen.getByTestId('chat-stall-quiet')).toHaveTextContent('Quiet for 2m 0s.');

    act(() => {
      vi.advanceTimersByTime(5_000);
    });
    expect(screen.getByTestId('chat-stall-quiet')).toHaveTextContent('Quiet for 2m 5s.');
  });
});
