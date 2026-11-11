import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { MessageVoiceNote, useMessageVoiceNote } from './MessageVoiceNote';

const state = vi.hoisted(() => ({
  message: {
    metadata: { custom: { extraMetadata: {} as unknown } },
    parts: [
      { type: 'text', text: 'hello ' },
      { type: 'text', text: 'there' },
    ],
  },
}));
vi.mock('@assistant-ui/react', () => ({
  useAuiState: (selector: (s: unknown) => unknown) => selector({ message: state.message }),
}));
vi.mock('../../components/analytics', () => ({ trackAnalyticsEvent: vi.fn() }));

function Probe() {
  const note = useMessageVoiceNote();
  return note ? (
    <MessageVoiceNote note={note} withTranscript />
  ) : (
    <span data-testid="plain">plain</span>
  );
}

describe('MessageVoiceNote', () => {
  it('renders nothing special for an ordinary message', () => {
    state.message.metadata.custom.extraMetadata = {};
    render(<Probe />);
    expect(screen.getByTestId('plain')).toBeInTheDocument();
  });

  it('renders a player with the message text as the transcript for a voice note row', () => {
    state.message.metadata.custom.extraMetadata = {
      voiceNote: { noteId: 'vn_1', durationMs: 5000 },
    };
    render(<Probe />);
    expect(screen.getByTestId('voice-note-player')).toBeInTheDocument();
    expect(screen.getByTestId('voice-note-time')).toHaveTextContent('0:05');
    screen.getByTestId('voice-note-transcript-toggle').click();
  });

  it('omits the transcript disclosure for an assistant reply', () => {
    state.message.metadata.custom.extraMetadata = { voiceNote: { noteId: 'reply-r' } };
    render(<MessageVoiceNote note={{ noteId: 'reply-r' }} withTranscript={false} />);
    expect(screen.queryByTestId('voice-note-transcript-toggle')).toBeNull();
  });
});
