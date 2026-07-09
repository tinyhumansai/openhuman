import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { ProcessingTranscriptView } from './ProcessingTranscriptView';

// Echo i18n: return the fallback when one is provided (so localized copy keys
// resolve to the English source we pass), otherwise the key itself. This lets
// us assert on the human strings carried on the failure payload.
vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string, fallback?: string) => fallback ?? key, locale: 'en' }),
}));

function failedEntry(overrides: Partial<ToolTimelineEntry> = {}): ToolTimelineEntry {
  return {
    id: 'call-1',
    name: 'read_file',
    round: 1,
    status: 'error',
    failure: {
      class: 'MissingPermission',
      category: 'NeedsUserConfirmation',
      recoverable: false,
      causePlain: 'No permission yet.',
      nextAction: 'Grant it, then retry.',
    },
    ...overrides,
  };
}

describe('ProcessingTranscriptView tool failure explanation', () => {
  it('renders the cause + next-action under a failed tool row', () => {
    // Empty transcript → single tool group over all entries (legacy path).
    render(<ProcessingTranscriptView transcript={[]} entries={[failedEntry()]} />);

    const failure = screen.getByTestId('processing-tool-failure');
    expect(failure).toBeTruthy();
    expect(failure.textContent).toContain('No permission yet.');
    expect(failure.textContent).toContain('Grant it, then retry.');
  });

  it('falls back to the plain English copy for an unrecognized class', () => {
    render(
      <ProcessingTranscriptView
        transcript={[]}
        entries={[
          failedEntry({
            failure: {
              class: 'SomethingBrandNew',
              category: 'Recoverable',
              recoverable: true,
              causePlain: 'Mystery cause.',
              nextAction: 'Mystery next.',
            },
          }),
        ]}
      />
    );

    const failure = screen.getByTestId('processing-tool-failure');
    expect(failure.textContent).toContain('Mystery cause.');
    expect(failure.textContent).toContain('Mystery next.');
  });

  it('does not render the failure block for a successful entry', () => {
    render(
      <ProcessingTranscriptView
        transcript={[]}
        entries={[failedEntry({ status: 'success', failure: undefined })]}
      />
    );
    expect(screen.queryByTestId('processing-tool-failure')).toBeNull();
  });
});
