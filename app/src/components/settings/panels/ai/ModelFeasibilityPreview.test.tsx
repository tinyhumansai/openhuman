import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { ModelRegistryEntry } from '../../../../utils/tauriCommands/config';
import { ModelFeasibilityPreview } from './ModelFeasibilityPreview';

const cachedModel: ModelRegistryEntry = {
  provider: 'demo',
  id: 'cache-capable',
  context_window: 64_000,
  vision: false,
  cost_per_1m_input: 10,
  cost_per_1m_cached_input: 1,
  cost_per_1m_output: 20,
};

describe('ModelFeasibilityPreview cached input controls', () => {
  it('accounts for cached tokens as a subset of estimated input, without modifying routing', () => {
    render(<ModelFeasibilityPreview models={[cachedModel]} />);
    fireEvent.change(screen.getByLabelText('Estimated input tokens'), {
      target: { value: '1000' },
    });
    fireEvent.change(screen.getByLabelText('Cached input tokens (within input)'), {
      target: { value: '800' },
    });
    fireEvent.change(screen.getByLabelText('Estimated output tokens'), {
      target: { value: '100' },
    });
    expect(screen.getByText('Est. $0.0048')).toBeTruthy();
    expect(screen.getByText(/Matches saved metadata; availability unverified/)).toBeTruthy();
  });

  it('requires cached input to be no greater than total input tokens', () => {
    render(<ModelFeasibilityPreview models={[cachedModel]} />);
    fireEvent.change(screen.getByLabelText('Estimated input tokens'), {
      target: { value: '1000' },
    });
    fireEvent.change(screen.getByLabelText('Cached input tokens (within input)'), {
      target: { value: '1001' },
    });
    expect(screen.getByRole('alert').textContent).toContain(
      'Cached input must be within total input tokens'
    );
  });
});

describe('ModelFeasibilityPreview catalog browsing', () => {
  const manyModels: ModelRegistryEntry[] = Array.from({ length: 17 }, (_, index) => ({
    ...cachedModel,
    id: 'candidate-' + index,
  }));

  it('reveals additional sorted results on demand, not only the first eight', () => {
    render(<ModelFeasibilityPreview models={manyModels} />);
    expect(screen.queryByText('demo/candidate-16')).toBeNull();
    expect(screen.queryByText('demo/candidate-9')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Show more models' }));
    expect(screen.getByText('demo/candidate-16')).toBeTruthy();
    expect(screen.queryByText('demo/candidate-9')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Show more models' }));
    expect(screen.getByText('demo/candidate-9')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Show more models' })).toBeNull();
  });

  it('filters the full catalog by model identity and selected fit status', () => {
    render(<ModelFeasibilityPreview models={manyModels} />);
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search suitability results' }), {
      target: { value: 'candidate-16' },
    });
    expect(screen.getByText('demo/candidate-16')).toBeTruthy();
    expect(screen.queryByText('demo/candidate-1')).toBeNull();
    fireEvent.change(screen.getByRole('combobox', { name: 'Filter suitability status' }), {
      target: { value: 'excluded' },
    });
    expect(screen.getByText('No models match these filters.')).toBeTruthy();
    fireEvent.change(screen.getByRole('combobox', { name: 'Filter suitability status' }), {
      target: { value: 'metadata_fit' },
    });
    expect(screen.getByText('demo/candidate-16')).toBeTruthy();
  });
});
