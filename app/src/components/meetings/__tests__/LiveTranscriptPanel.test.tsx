import { cleanup, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { LiveTranscriptPanel } from '../LiveTranscriptPanel';

describe('LiveTranscriptPanel (#4304)', () => {
  afterEach(() => cleanup());

  it('shows an empty/waiting state when there are no turns', () => {
    renderWithProviders(<LiveTranscriptPanel turns={[]} partialIndex={null} />);
    expect(screen.getByText(/waiting for speech/i)).toBeInTheDocument();
    // Heading is always present.
    expect(screen.getByText(/live transcript/i)).toBeInTheDocument();
  });

  it('renders turns with their role label and content', () => {
    renderWithProviders(
      <LiveTranscriptPanel
        turns={[
          { role: 'user', content: 'Hello team' },
          { role: 'assistant', content: 'Hi there' },
        ]}
        partialIndex={null}
      />
    );
    expect(screen.getByText('Hello team')).toBeInTheDocument();
    expect(screen.getByText('Hi there')).toBeInTheDocument();
    expect(screen.queryByText(/waiting for speech/i)).not.toBeInTheDocument();
  });

  it('renders the inline [Speaker] tag as a label, not raw bracket text', () => {
    renderWithProviders(
      <LiveTranscriptPanel
        turns={[{ role: 'user', content: '[Alice] hello there' }]}
        partialIndex={null}
      />
    );
    // Speaker is pulled out of the content and shown as a label.
    expect(screen.getByText('Alice:')).toBeInTheDocument();
    expect(screen.getByText('hello there')).toBeInTheDocument();
    // The raw bracketed form is not shown.
    expect(screen.queryByText(/\[Alice\]/)).not.toBeInTheDocument();
  });

  it('greys the partial line at partialIndex', () => {
    renderWithProviders(
      <LiveTranscriptPanel
        turns={[
          { role: 'user', content: 'final line' },
          { role: 'user', content: 'partial line' },
        ]}
        partialIndex={1}
      />
    );
    const partial = screen.getByText('partial line').closest('p');
    const settled = screen.getByText('final line').closest('p');
    expect(partial?.className).toContain('text-content-faint');
    expect(partial?.className).toContain('italic');
    expect(settled?.className).not.toContain('text-content-faint');
  });
});
