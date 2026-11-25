import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { AssistantUiToolCallCard, formatElapsed } from './AssistantUiToolCall';

const SEARCH_TEXT = [
  'Search results for: rust async traits (via Exa)',
  '1. Async fn in traits',
  '   https://blog.rust-lang.org/async-fn',
  '   Stable in 1.75.',
  '2. Bad link',
  '   javascript:alert(1)',
].join('\n');

describe('AssistantUiToolCallCard', () => {
  it('renders a web search through the web-search element, visible without expanding', () => {
    render(
      <AssistantUiToolCallCard
        toolName="web_search_tool"
        args={{ query: 'rust async traits' }}
        result={SEARCH_TEXT}
        status="success"
        elapsedMs={1840}
      />
    );
    const results = screen.getByTestId('web-search-results');
    expect(results).toHaveTextContent('rust async traits');
    expect(results).toHaveTextContent('Found 1 result · via Exa');
    const hits = within(results).getAllByTestId('web-search-hit');
    expect(hits).toHaveLength(1);
    expect(hits[0]).toHaveAttribute('href', 'https://blog.rust-lang.org/async-fn');
    // The javascript: hit never becomes a link or a row.
    expect(results).not.toHaveTextContent('Bad link');
    expect(screen.getByTestId('tool-call-elapsed')).toHaveTextContent('1.8s');
  });

  it('prefers the structured search payload the core attaches', () => {
    render(
      <AssistantUiToolCallCard
        toolName="web_search_tool"
        args={{ query: 'q' }}
        result="unparseable"
        structured={{
          kind: 'web_search',
          query: 'q',
          provider: 'Parallel',
          results: [{ title: 'From payload', url: 'https://a.dev/x' }],
        }}
        status="success"
      />
    );
    expect(screen.getByTestId('web-search-results')).toHaveTextContent('From payload');
    expect(screen.getByTestId('web-search-results')).toHaveTextContent('via Parallel');
  });

  it('renders a web answer with its text, citations and fallback note', () => {
    render(
      <AssistantUiToolCallCard
        toolName="web_answer_tool"
        args={{ query: 'who won' }}
        result="Answer for: who won (via Gemini, after Exa)"
        structured={{
          kind: 'web_search',
          query: 'who won',
          provider: 'Gemini',
          role: 'answer',
          answer: 'Team A won the final.',
          citations: [
            { url: 'https://news.example/final', title: 'Final report' },
            { url: 'javascript:alert(1)', title: 'Bad' },
          ],
          fallback_from: ['Exa'],
          results: [],
        }}
        status="success"
      />
    );
    expect(screen.getByTestId('web-search-answer')).toHaveTextContent('Team A won the final.');
    const results = screen.getByTestId('web-search-results');
    expect(results).toHaveTextContent('1 source · via Gemini, after Exa');
    const hits = within(results).getAllByTestId('web-search-hit');
    expect(hits).toHaveLength(1);
    expect(hits[0]).toHaveAttribute('href', 'https://news.example/final');
    expect(screen.getByRole('button', { name: /Asked the web/ })).toBeInTheDocument();
  });

  it('shows an insufficient-balance hint when managed search runs out of balance', () => {
    render(
      <AssistantUiToolCallCard
        toolName="web_search_tool"
        args={{ query: 'q' }}
        result="web search failed: your TinyHumans balance is too low"
        status="error"
      />
    );
    expect(screen.getByTestId('web-search-balance-hint')).toHaveTextContent(
      /balance is too low for included search/
    );
  });

  it('does not turn an empty tool result into a fabricated failure diagnostic', () => {
    render(
      <AssistantUiToolCallCard
        toolName="unrecognized_tool"
        result=""
        status="error"
        failure={{
          class: 'Unknown',
          category: 'Failed',
          recoverable: true,
          causePlain: 'Underlying cause unavailable',
          nextAction: 'Try again',
        }}
      />
    );
    const card = screen.getByTestId('assistant-ui-tool-failure');
    expect(card).toHaveTextContent('Something went wrong with this action.');
    expect(card).not.toHaveTextContent('Why: No output');
  });

  it('shows no balance hint for other failures', () => {
    render(
      <AssistantUiToolCallCard
        toolName="web_search_tool"
        args={{ query: 'q' }}
        result="timed out"
        status="error"
      />
    );
    expect(screen.queryByTestId('web-search-balance-hint')).toBeNull();
  });

  it('shows a live elapsed clock while a tool runs', () => {
    vi.useFakeTimers();
    try {
      const { rerender } = render(
        <AssistantUiToolCallCard toolName="file_read" args={{ path: 'a.ts' }} status="running" />
      );
      expect(screen.queryByTestId('tool-call-elapsed-live')).toBeNull();
      act(() => {
        vi.advanceTimersByTime(2_000);
      });
      expect(screen.getByTestId('tool-call-elapsed-live')).toHaveTextContent('2.0s');
      rerender(
        <AssistantUiToolCallCard
          toolName="file_read"
          args={{ path: 'a.ts' }}
          status="success"
          result="x"
          elapsedMs={2_300}
        />
      );
      expect(screen.queryByTestId('tool-call-elapsed-live')).toBeNull();
      expect(screen.getByTestId('tool-call-elapsed')).toHaveTextContent('2.3s');
    } finally {
      vi.useRealTimers();
    }
  });

  it('swaps the label tense as the call settles', () => {
    const { rerender } = render(
      <AssistantUiToolCallCard toolName="file_read" args={{ path: 'a.ts' }} status="running" />
    );
    const card = screen.getByTestId('assistant-ui-tool-call');
    expect(card).toHaveAttribute('data-outcome', 'running');
    expect(within(card).getByRole('button', { name: /Reading file/ })).toBeInTheDocument();
    rerender(
      <AssistantUiToolCallCard
        toolName="file_read"
        args={{ path: 'a.ts' }}
        status="success"
        result="x"
      />
    );
    expect(card).toHaveAttribute('data-outcome', 'success');
    expect(within(card).getByRole('button', { name: /Read file/ })).toBeInTheDocument();
  });

  it('spells out states that need attention and keeps the rest for screen readers', () => {
    render(
      <>
        <AssistantUiToolCallCard toolName="grep" status="error" result="bad regex" />
        <AssistantUiToolCallCard toolName="shell" status="success" result="ok" />
      </>
    );
    const [failed, done] = screen.getAllByTestId('tool-call-status');
    expect(failed).toHaveTextContent('failed');
    expect(failed).not.toHaveClass('sr-only');
    expect(done).toHaveTextContent('done');
    expect(done).toHaveClass('sr-only');
  });

  it('expands an edit into the code-diff element and a read into a code block', async () => {
    render(
      <>
        <AssistantUiToolCallCard
          toolName="edit"
          args={{ path: 'a.ts', old_string: 'old', new_string: 'new' }}
          status="success"
          result="ok"
        />
        <AssistantUiToolCallCard
          toolName="file_read"
          args={{ path: 'src/b.rs' }}
          status="success"
          result="fn main() {}"
        />
      </>
    );
    const [edit, read] = screen.getAllByTestId('assistant-ui-tool-call');
    await userEvent.click(within(edit).getByRole('button', { name: /Edited file/ }));
    const diff = screen.getByTestId('tool-body-file-diff');
    expect(diff).toHaveTextContent('+1');
    expect(diff).toHaveTextContent('old');
    await userEvent.click(within(read).getByRole('button', { name: /Read file/ }));
    expect(screen.getByTestId('tool-body-file')).toHaveTextContent('fn main() {}');
  });

  it('names a connected-app action by its app, never the slug', () => {
    render(<AssistantUiToolCallCard toolName="GMAIL_SEND_EMAIL" status="success" result="ok" />);
    const card = screen.getByTestId('assistant-ui-tool-call');
    expect(card).toHaveTextContent('Used Gmail');
    expect(card).toHaveTextContent('Send email');
    expect(card).not.toHaveTextContent('GMAIL_SEND_EMAIL');
  });
});

describe('formatElapsed', () => {
  it('formats milliseconds, seconds and minutes', () => {
    expect(formatElapsed(850)).toBe('850ms');
    expect(formatElapsed(1840)).toBe('1.8s');
    expect(formatElapsed(75_000)).toBe('1m 15s');
  });
});
