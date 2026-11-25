import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { ToolFailureExplanation } from '../../../store/chatRuntimeSlice';
import { ToolFailureCard } from './ToolFailureCard';

const failure: ToolFailureExplanation = {
  class: 'MissingPermission',
  category: 'BlockedByPolicy',
  recoverable: false,
  causePlain: 'The app needs calendar access.',
  nextAction: 'Grant calendar access and try again.',
};

describe('ToolFailureCard', () => {
  it('renders the failure through the vendored tool-error element', () => {
    render(<ToolFailureCard toolName="calendar_create_event" failure={failure} />);

    const card = screen.getByTestId('assistant-ui-tool-failure');
    expect(card).toHaveTextContent('calendar_create_event');
    expect(card).toHaveTextContent(/grant the permission/i);
  });

  it('falls back to the failure class as the target when none is given', () => {
    render(<ToolFailureCard toolName="shell" failure={failure} />);

    expect(screen.getByTestId('assistant-ui-tool-failure')).toHaveTextContent('MissingPermission');
  });

  it('prefers an explicit target over the failure class', () => {
    render(<ToolFailureCard toolName="shell" target="rm -rf /tmp/x" failure={failure} />);

    expect(screen.getByTestId('assistant-ui-tool-failure')).toHaveTextContent('rm -rf /tmp/x');
  });

  it('localizes unknown failure copy when the core provides no diagnostic', () => {
    render(<ToolFailureCard toolName="cron" failure={{ ...failure, class: 'Unknown' }} />);
    const card = screen.getByTestId('assistant-ui-tool-failure');
    expect(card).toHaveTextContent('Something went wrong with this action.');
    expect(card).toHaveTextContent(
      'Try again; if it keeps failing, run diagnostics from Settings.'
    );
    expect(card).not.toHaveTextContent('The app needs calendar access.');
  });

  it('shows an unknown failure diagnostic without invented attempts or unavailable actions', () => {
    render(
      <ToolFailureCard
        toolName="cron"
        failure={{ ...failure, class: 'Unknown' }}
        diagnostic="Command execution is unavailable."
      />
    );
    const card = screen.getByTestId('assistant-ui-tool-failure');
    expect(card).toHaveTextContent('Command execution is unavailable.');
    expect(card).toHaveTextContent(
      'Try again; if it keeps failing, run diagnostics from Settings.'
    );
    expect(card).not.toHaveTextContent('Grant calendar access and try again.');
    expect(card).not.toHaveTextContent('Unknown');
    expect(card).not.toHaveTextContent('1/1');
    expect(screen.queryByRole('button', { name: /retry|skip/i })).not.toBeInTheDocument();
  });
});
