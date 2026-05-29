import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ModelCouncilResult } from '../../services/api/modelCouncilApi';
import ModelCouncilTab from './ModelCouncilTab';

const mockRunCouncil = vi.fn();
vi.mock('../../services/api/modelCouncilApi', () => ({
  modelCouncilApi: { runCouncil: (...args: unknown[]) => mockRunCouncil(...args) },
}));

const RESULT: ModelCouncilResult = {
  question: 'What is the capital of France?',
  members: [
    { model: 'model-a', response: 'Paris is the capital.', error: null },
    { model: 'model-b', response: null, error: 'rate limited' },
  ],
  chair_model: 'chair-model',
  synthesis: 'Both that answered agree: Paris. One seat failed.',
};

const fillValidForm = () => {
  fireEvent.change(screen.getByLabelText('Question'), {
    target: { value: 'What is the capital of France?' },
  });
  fireEvent.change(screen.getByLabelText('Member model 1'), { target: { value: 'model-a' } });
  fireEvent.change(screen.getByLabelText('Member model 2'), { target: { value: 'model-b' } });
  fireEvent.change(screen.getByLabelText('Chair model'), { target: { value: 'chair-model' } });
};

describe('ModelCouncilTab', () => {
  beforeEach(() => {
    mockRunCouncil.mockReset();
  });

  it('renders the compose surface with two member rows by default', () => {
    render(<ModelCouncilTab />);
    expect(screen.getByText('Model Council')).toBeInTheDocument();
    expect(screen.getByLabelText('Question')).toBeInTheDocument();
    expect(screen.getByLabelText('Member model 1')).toBeInTheDocument();
    expect(screen.getByLabelText('Member model 2')).toBeInTheDocument();
    expect(screen.getByLabelText('Chair model')).toBeInTheDocument();
  });

  it('disables Convene until question + a member + chair are all filled', () => {
    render(<ModelCouncilTab />);
    const run = screen.getByRole('button', { name: 'Convene council' });
    expect(run).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Question'), { target: { value: 'q' } });
    expect(run).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Member model 1'), { target: { value: 'm' } });
    expect(run).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Chair model'), { target: { value: 'c' } });
    expect(run).not.toBeDisabled();
  });

  it('adds member rows up to the max of 5 and stops', () => {
    render(<ModelCouncilTab />);
    const add = screen.getByRole('button', { name: '+ Add model' });
    fireEvent.click(add); // 3
    fireEvent.click(add); // 4
    fireEvent.click(add); // 5
    expect(screen.getByLabelText('Member model 5')).toBeInTheDocument();
    expect(add).toBeDisabled();
    expect(screen.queryByLabelText('Member model 6')).not.toBeInTheDocument();
  });

  it('removes member rows but never below one', () => {
    render(<ModelCouncilTab />);
    // Two rows initially; remove one → one left, remove button then disabled.
    fireEvent.click(screen.getByRole('button', { name: 'Remove member model 2' }));
    expect(screen.queryByLabelText('Member model 2')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Remove member model 1' })).toBeDisabled();
  });

  it('runs the council and renders member answers side-by-side + the synthesis', async () => {
    mockRunCouncil.mockResolvedValueOnce(RESULT);
    render(<ModelCouncilTab />);
    fillValidForm();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });
    expect(mockRunCouncil).toHaveBeenCalledWith({
      question: 'What is the capital of France?',
      member_models: ['model-a', 'model-b'],
      chair_model: 'chair-model',
    });
    await waitFor(() => {
      expect(screen.getByText('Council results')).toBeInTheDocument();
    });
    // Member A answered; Member B failed.
    expect(screen.getByText('Paris is the capital.')).toBeInTheDocument();
    expect(screen.getByText('rate limited')).toBeInTheDocument();
    expect(screen.getByText('Answered')).toBeInTheDocument();
    expect(screen.getByText('Failed')).toBeInTheDocument();
    // Synthesis from the chair.
    expect(
      screen.getByText('Both that answered agree: Paris. One seat failed.')
    ).toBeInTheDocument();
    expect(screen.getByText('by chair-model')).toBeInTheDocument();
  });

  it('trims whitespace and drops blank member rows before calling the API', async () => {
    mockRunCouncil.mockResolvedValueOnce(RESULT);
    render(<ModelCouncilTab />);
    fireEvent.change(screen.getByLabelText('Question'), { target: { value: '  hi  ' } });
    fireEvent.change(screen.getByLabelText('Member model 1'), { target: { value: ' model-a ' } });
    // leave member 2 blank
    fireEvent.change(screen.getByLabelText('Chair model'), { target: { value: ' chair ' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });
    expect(mockRunCouncil).toHaveBeenCalledWith({
      question: 'hi',
      member_models: ['model-a'],
      chair_model: 'chair',
    });
  });

  it('surfaces an error alert when the council run fails', async () => {
    mockRunCouncil.mockRejectedValueOnce(new Error('all member models failed to respond'));
    render(<ModelCouncilTab />);
    fillValidForm();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });
    await waitFor(() => {
      const alert = screen.getByRole('alert');
      expect(alert.textContent).toMatch(/all member models failed to respond/);
    });
    // No results section on failure.
    expect(screen.queryByText('Council results')).not.toBeInTheDocument();
  });
});
