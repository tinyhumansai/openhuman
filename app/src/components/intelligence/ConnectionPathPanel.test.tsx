import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ConnectionPathResult } from '../../lib/memory/connectionPath';
import ConnectionPathPanel from './ConnectionPathPanel';

function res(overrides: Partial<ConnectionPathResult> = {}): ConnectionPathResult {
  return { found: true, source: 'A', target: 'C', hops: [], length: 0, reason: 'ok', ...overrides };
}

describe('<ConnectionPathPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<ConnectionPathPanel result={null} hasGraph loading />);
    expect(screen.getByTestId('connection-path-loading')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(
      <ConnectionPathPanel
        result={null}
        hasGraph={false}
        error="graph unavailable"
        onRetry={onRetry}
      />
    );
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
  });

  it('shows the empty state when there is no graph', () => {
    render(<ConnectionPathPanel result={null} hasGraph={false} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('prompts to pick two entities when the graph is present but nothing is chosen', () => {
    render(<ConnectionPathPanel result={null} hasGraph />);
    expect(
      screen.getByText('Pick two entities to trace how they are connected.')
    ).toBeInTheDocument();
  });

  it('renders a found path as a chain of nodes with predicates', () => {
    render(
      <ConnectionPathPanel
        hasGraph
        result={res({
          length: 2,
          hops: [
            { from: 'A', to: 'B', predicate: 'knows', forward: true },
            { from: 'B', to: 'C', predicate: 'likes', forward: false },
          ],
        })}
      />
    );
    expect(screen.getByText('Shortest path')).toBeInTheDocument();
    expect(screen.getByText('A')).toBeInTheDocument();
    expect(screen.getByText('B')).toBeInTheDocument();
    expect(screen.getByText('C')).toBeInTheDocument();
    expect(screen.getByText('knows →')).toBeInTheDocument();
    expect(screen.getByText('← likes')).toBeInTheDocument();
  });

  it('reports when no connection exists', () => {
    render(
      <ConnectionPathPanel
        hasGraph
        result={res({ found: false, source: 'A', target: 'Z', reason: 'no-path' })}
      />
    );
    expect(screen.getByText('No connection found between "A" and "Z".')).toBeInTheDocument();
  });

  it('reports a missing endpoint', () => {
    render(
      <ConnectionPathPanel
        hasGraph
        result={res({ found: false, source: 'A', target: 'Q', reason: 'missing-target' })}
      />
    );
    expect(screen.getByText('"Q" is not in the graph.')).toBeInTheDocument();
  });

  it('prompts for two different entities when the endpoints are identical', () => {
    render(
      <ConnectionPathPanel
        hasGraph
        result={res({ found: true, source: 'A', target: 'A', reason: 'same' })}
      />
    );
    expect(screen.getByText('Pick two different entities.')).toBeInTheDocument();
  });
});
