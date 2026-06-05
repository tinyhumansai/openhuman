import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { CouncilDefinition } from '../../services/api/councilRegistryApi';
import type { CouncilMemberResult, ModelCouncilResult } from '../../services/api/modelCouncilApi';
import ModelCouncilTab from './ModelCouncilTab';

const mockListCouncils = vi.fn();
const mockUpsertCouncil = vi.fn();
const mockDeleteCouncil = vi.fn();
const mockAnswerMember = vi.fn();
const mockSynthesizeCouncil = vi.fn();
const mockLoadAISettings = vi.fn();
const mockLoadLocalProviderSnapshot = vi.fn();
const mockListProviderModels = vi.fn();
const mockDispatch = vi.fn();

const mockState = {
  agentProfiles: {
    profiles: [
      {
        id: 'default',
        name: 'Default Agent',
        description: 'Default',
        agentId: 'openhuman.default',
        modelOverride: 'profile-model',
        builtIn: true,
      },
      {
        id: 'critic',
        name: 'Critic',
        description: 'Finds gaps',
        agentId: 'critic-agent',
        modelOverride: 'critic-model',
        builtIn: false,
      },
    ],
    activeProfileId: 'default',
    status: 'idle',
    error: null,
  },
};

vi.mock('../../services/api/modelCouncilApi', () => ({
  modelCouncilApi: {
    answerMember: (...args: unknown[]) => mockAnswerMember(...args),
    synthesizeCouncil: (...args: unknown[]) => mockSynthesizeCouncil(...args),
  },
}));

vi.mock('../../services/api/councilRegistryApi', () => ({
  councilRegistryApi: {
    list: (...args: unknown[]) => mockListCouncils(...args),
    upsert: (...args: unknown[]) => mockUpsertCouncil(...args),
    delete: (...args: unknown[]) => mockDeleteCouncil(...args),
  },
}));

vi.mock('../../services/api/aiSettingsApi', () => ({
  loadAISettings: (...args: unknown[]) => mockLoadAISettings(...args),
  loadLocalProviderSnapshot: (...args: unknown[]) => mockLoadLocalProviderSnapshot(...args),
  listProviderModels: (...args: unknown[]) => mockListProviderModels(...args),
}));

vi.mock('../../store/hooks', () => ({
  useAppDispatch: () => mockDispatch,
  useAppSelector: (selector: (state: typeof mockState) => unknown) => selector(mockState),
}));

vi.mock('../../features/human/Mascot', () => ({
  RiveMascot: ({ face }: { face?: string }) => <div data-testid="rive-mascot" data-face={face} />,
  getMascotPalette: () => ({ bodyFill: '#F7D145', neckShadowColor: '#B23C05' }),
  hexToArgbInt: () => 0xfff7d145,
}));

const RESULT: ModelCouncilResult = {
  question: 'What is the capital of France?',
  members: [
    { model: 'gpt-5.2', response: 'Paris is the capital.', error: null },
    { model: 'critic-model', response: null, error: 'rate limited' },
  ],
  chair_model: 'claude-opus-4-8',
  synthesis: 'Both that answered agree: Paris. One seat failed.',
};

const DEFAULT_MEMBERS: CouncilMemberResult[] = [
  { model: 'reasoning-v1', response: 'Paris is the capital.', error: null },
  { model: 'reasoning-v1', response: 'France uses Paris as its capital.', error: null },
  { model: 'reasoning-v1', response: 'The answer is Paris.', error: null },
];

const DEFAULT_COUNCIL: CouncilDefinition = {
  id: 'default-council',
  name: 'Default council',
  description: 'Balanced analyst, builder, and skeptic jury.',
  jury_count: 3,
  debate_rounds: 3,
  seats: [
    {
      id: 0,
      mode: 'default',
      profile_id: '',
      name: 'Analyst',
      model: 'reasoning-v1',
      brief: 'Evidence, assumptions, and risk.',
    },
    {
      id: 1,
      mode: 'default',
      profile_id: '',
      name: 'Builder',
      model: 'reasoning-v1',
      brief: 'Practical implementation path.',
    },
    {
      id: 2,
      mode: 'default',
      profile_id: '',
      name: 'Skeptic',
      model: 'reasoning-v1',
      brief: 'Failure modes and missing context.',
    },
  ],
  judge: { mode: 'default', profile_id: '', name: 'Chief Judge', model: 'reasoning-v1' },
  shared_reasoning: [
    '# Shared reasoning',
    '- Claims the council agrees on:',
    '- Open disagreements:',
    '- Evidence or constraints to preserve:',
    '- Judge synthesis notes:',
  ].join('\n'),
  created_at_ms: 1,
  updated_at_ms: 1,
};

const fillQuestion = () => {
  fireEvent.change(screen.getByLabelText('Question'), {
    target: { value: 'What is the capital of France?' },
  });
};

const mockProgressiveSuccess = (members: CouncilMemberResult[] = DEFAULT_MEMBERS) => {
  mockAnswerMember.mockImplementation(async ({ model }: { model: string }) => {
    const index = mockAnswerMember.mock.calls.length - 1;
    return members[index] ?? { model, response: `answer ${index + 1}`, error: null };
  });
  mockSynthesizeCouncil.mockResolvedValue(RESULT);
};

const renderCouncilList = async () => {
  render(<ModelCouncilTab />);
  await screen.findByRole('button', { name: 'Open council' });
};

const renderOpenCouncil = async () => {
  await renderCouncilList();
  fireEvent.click(screen.getByRole('button', { name: 'Open council' }));
  await screen.findByLabelText('Question');
};

const renderEditCouncil = async () => {
  await renderOpenCouncil();
  fireEvent.click(screen.getByRole('button', { name: 'Edit current council' }));
  await screen.findByLabelText('Council name');
};

const saveCouncilSettings = async () => {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Save council' }));
  });
  await screen.findByLabelText('Question');
};

describe('ModelCouncilTab', () => {
  beforeEach(() => {
    mockListCouncils.mockReset();
    mockUpsertCouncil.mockReset();
    mockDeleteCouncil.mockReset();
    mockAnswerMember.mockReset();
    mockSynthesizeCouncil.mockReset();
    mockLoadAISettings.mockReset();
    mockLoadLocalProviderSnapshot.mockReset();
    mockListProviderModels.mockReset();
    mockDispatch.mockReset();
    mockListCouncils.mockResolvedValue([DEFAULT_COUNCIL]);
    mockUpsertCouncil.mockImplementation(async council => ({
      ...council,
      id: council.id || 'saved',
    }));
    mockDeleteCouncil.mockResolvedValue(true);
    mockLoadAISettings.mockResolvedValue({
      cloudProviders: [
        {
          id: 'openai-id',
          slug: 'openai',
          label: 'OpenAI',
          endpoint: 'https://api.openai.com/v1',
          auth_style: 'bearer',
          has_api_key: true,
        },
        {
          id: 'anthropic-id',
          slug: 'anthropic',
          label: 'Anthropic',
          endpoint: 'https://api.anthropic.com/v1',
          auth_style: 'anthropic',
          has_api_key: false,
        },
      ],
      routing: {},
    });
    mockLoadLocalProviderSnapshot.mockResolvedValue({
      status: null,
      diagnostics: null,
      presets: null,
      installedModels: [{ name: 'llama3.2:latest', chat_capable: true }],
    });
    mockListProviderModels.mockImplementation(async (provider: string) => {
      if (provider === 'openhuman') return [{ id: 'managed-reasoning' }];
      if (provider === 'openai') return [{ id: 'gpt-4o' }, { id: 'gpt-4o-mini' }];
      return [];
    });
  });

  it('renders the council list first, then opens the default council', async () => {
    await renderCouncilList();

    expect(screen.getByText('Councils')).toBeInTheDocument();
    expect(screen.getByText('Default council')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Open council' }));

    await screen.findByLabelText('Question');
    expect(screen.getByText('Default council')).toBeInTheDocument();
    expect(screen.queryByText('Council settings')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Debate turns')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Shared reasoning file')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Convene council' })).toBeInTheDocument();
  });

  it('allows the default council to be deleted from the persisted registry', async () => {
    await renderCouncilList();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Delete Default council' }));
    });

    expect(mockDeleteCouncil).toHaveBeenCalledWith('default-council');
    expect(screen.queryByText('Default council')).not.toBeInTheDocument();
    expect(screen.getByText('No councils yet. Add one to get started.')).toBeInTheDocument();
  });

  it('uses the jury count setting to resize the roster up to five', async () => {
    await renderEditCouncil();

    expect(screen.queryByLabelText('Question')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Convene council' })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '5' }));

    expect(screen.getAllByTestId('rive-mascot')).toHaveLength(5);
    expect(screen.getAllByText('Juror 5')).toHaveLength(2);
    expect(screen.getByLabelText('Juror 5 name')).toBeInTheDocument();
  });

  it('disables Convene until a question is filled because seats and judge have defaults', async () => {
    await renderOpenCouncil();

    const run = screen.getByRole('button', { name: 'Convene council' });
    expect(run).toBeDisabled();
    fillQuestion();
    expect(run).not.toBeDisabled();
  });

  it('shows mascot deliberation and agent thoughts while the council is running', async () => {
    let resolveFirst: (value: CouncilMemberResult) => void = () => {};
    let resolveSecond: (value: CouncilMemberResult) => void = () => {};
    let resolveThird: (value: CouncilMemberResult) => void = () => {};
    let resolveSynthesis: (value: ModelCouncilResult) => void = () => {};
    mockAnswerMember
      .mockImplementation(async ({ model }: { model: string }) => ({
        model,
        response: `follow-up thought ${mockAnswerMember.mock.calls.length}`,
        error: null,
      }))
      .mockReturnValueOnce(
        new Promise<CouncilMemberResult>(resolve => {
          resolveFirst = resolve;
        })
      )
      .mockReturnValueOnce(
        new Promise<CouncilMemberResult>(resolve => {
          resolveSecond = resolve;
        })
      )
      .mockReturnValueOnce(
        new Promise<CouncilMemberResult>(resolve => {
          resolveThird = resolve;
        })
      );
    mockSynthesizeCouncil.mockReturnValueOnce(
      new Promise<ModelCouncilResult>(resolve => {
        resolveSynthesis = resolve;
      })
    );
    await renderOpenCouncil();
    fillQuestion();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });

    expect(screen.getByText('Council deliberation')).toBeInTheDocument();
    expect(screen.getAllByText('Thinking')).toHaveLength(3);
    expect(screen.getByText('Judge')).toBeInTheDocument();
    expect(
      screen.getByText(/Waiting for juror answers, then reading the shared reasoning file/)
    ).toBeInTheDocument();
    expect(screen.getAllByTestId('rive-mascot')).toHaveLength(4);
    expect(screen.getAllByTestId('rive-mascot')[0]).toHaveAttribute('data-face', 'thinking');

    await act(async () => {
      resolveFirst({
        model: 'reasoning-v1',
        response: 'First juror live thought: Paris.',
        error: null,
      });
    });

    expect(screen.getByText('First juror live thought: Paris.')).toBeInTheDocument();
    expect(screen.getByText('Round 1')).toBeInTheDocument();
    expect(screen.getAllByText('Thinking')).toHaveLength(2);
    expect(screen.getByText('Answered')).toBeInTheDocument();
    expect(screen.getByText('Judge')).toBeInTheDocument();

    await act(async () => {
      resolveSecond({ model: 'reasoning-v1', response: 'Second juror agrees.', error: null });
      resolveThird({ model: 'reasoning-v1', response: 'Third juror agrees.', error: null });
    });

    await waitFor(() => {
      expect(screen.getByText('Synthesizing')).toBeInTheDocument();
    });

    await act(async () => {
      resolveSynthesis(RESULT);
    });

    await waitFor(() => {
      expect(screen.queryByText('Council deliberation')).not.toBeInTheDocument();
    });
  });

  it('streams failed juror status without blocking other juror thoughts', async () => {
    let resolveFirst: (value: CouncilMemberResult) => void = () => {};
    let resolveSecond: (value: CouncilMemberResult) => void = () => {};
    let resolveThird: (value: CouncilMemberResult) => void = () => {};
    mockAnswerMember
      .mockImplementation(async ({ model }: { model: string }) => ({
        model,
        response: `follow-up answer ${mockAnswerMember.mock.calls.length}`,
        error: null,
      }))
      .mockReturnValueOnce(
        new Promise<CouncilMemberResult>(resolve => {
          resolveFirst = resolve;
        })
      )
      .mockReturnValueOnce(
        new Promise<CouncilMemberResult>(resolve => {
          resolveSecond = resolve;
        })
      )
      .mockReturnValueOnce(
        new Promise<CouncilMemberResult>(resolve => {
          resolveThird = resolve;
        })
      );
    mockSynthesizeCouncil.mockResolvedValueOnce(RESULT);
    await renderOpenCouncil();
    fillQuestion();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });

    await act(async () => {
      resolveFirst({ model: 'reasoning-v1', response: null, error: 'rate limited' });
    });

    expect(screen.getByText('rate limited')).toBeInTheDocument();
    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(screen.getAllByText('Thinking')).toHaveLength(2);

    await act(async () => {
      resolveSecond({ model: 'reasoning-v1', response: 'Second juror answer.', error: null });
      resolveThird({ model: 'reasoning-v1', response: 'Third juror answer.', error: null });
    });

    await waitFor(() => {
      expect(mockSynthesizeCouncil).toHaveBeenCalledWith({
        question: expect.any(String),
        members: [
          {
            model: 'reasoning-v1',
            response: expect.stringContaining('[failed: rate limited]'),
            error: null,
          },
          {
            model: 'reasoning-v1',
            response: expect.stringContaining('Second juror answer.'),
            error: null,
          },
          {
            model: 'reasoning-v1',
            response: expect.stringContaining('Third juror answer.'),
            error: null,
          },
        ],
        chair_model: 'reasoning-v1',
      });
    });
  });

  it('appends juror turns to the shared scratchpad before the next debate round', async () => {
    mockAnswerMember.mockImplementation(async ({ model }: { model: string }) => ({
      model,
      response: `round ${mockAnswerMember.mock.calls.length} update`,
      error: null,
    }));
    mockSynthesizeCouncil.mockResolvedValueOnce(RESULT);
    await renderOpenCouncil();
    fillQuestion();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });

    await waitFor(() => {
      expect(mockSynthesizeCouncil).toHaveBeenCalled();
    });
    expect(mockAnswerMember.mock.calls[3][0].question).toContain('Round 1 updates');
    expect(mockAnswerMember.mock.calls[3][0].question).toContain('round 1 update');
  });

  it('lets a juror model be selected from routing hints', async () => {
    mockProgressiveSuccess();
    await renderEditCouncil();

    fireEvent.click(screen.getByLabelText('Member model 1'));
    expect(screen.getByRole('dialog', { name: 'Member model 1' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Reasoning/ }));
    await saveCouncilSettings();
    fillQuestion();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });

    expect(mockAnswerMember.mock.calls.map(call => call[0].model).slice(0, 3)).toEqual([
      'hint:reasoning',
      'reasoning-v1',
      'reasoning-v1',
    ]);
  });

  it('enables provider and model dropdowns only after choosing Custom', async () => {
    await renderEditCouncil();

    fireEvent.click(screen.getByLabelText('Member model 1'));
    const dialog = screen.getByRole('dialog', { name: 'Member model 1' });
    const providerSelect = within(dialog).getByLabelText('Model provider');
    const modelSelect = within(dialog).getByLabelText('Model id');

    expect(providerSelect).toBeDisabled();
    expect(modelSelect).toBeDisabled();

    fireEvent.click(within(dialog).getByRole('button', { name: /Provider \+ model/ }));

    await waitFor(() => expect(providerSelect).not.toBeDisabled());
    expect(
      within(providerSelect).getByRole('option', { name: 'Managed (openhuman)' })
    ).toBeInTheDocument();
    expect(
      within(providerSelect).getByRole('option', { name: 'OpenAI (openai)' })
    ).toBeInTheDocument();
    expect(
      within(providerSelect).queryByRole('option', { name: 'Anthropic (anthropic)' })
    ).not.toBeInTheDocument();

    await waitFor(() => expect(modelSelect).not.toBeDisabled());
    expect(
      within(modelSelect).getByRole('option', { name: 'managed-reasoning' })
    ).toBeInTheDocument();

    fireEvent.change(providerSelect, { target: { value: 'openai' } });
    await waitFor(() => {
      expect(within(modelSelect).getByRole('option', { name: 'gpt-4o' })).toBeInTheDocument();
    });
    fireEvent.change(modelSelect, { target: { value: 'gpt-4o' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Use provider model' }));

    expect(screen.getByLabelText('Member model 1')).toHaveTextContent('openai:gpt-4o');
  });

  it('lets a council seat use a saved profile and submits that profile model', async () => {
    mockProgressiveSuccess();
    await renderEditCouncil();

    const firstSeat = screen.getByLabelText('Juror 1 name').closest('article');
    expect(firstSeat).not.toBeNull();
    fireEvent.click(within(firstSeat as HTMLElement).getByRole('tab', { name: 'Profile' }));
    fireEvent.change(screen.getByLabelText('Juror 1 profile'), { target: { value: 'critic' } });
    await saveCouncilSettings();
    fillQuestion();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });

    expect(mockAnswerMember.mock.calls.map(call => call[0].model)).toEqual([
      'critic-model',
      'reasoning-v1',
      'reasoning-v1',
      'critic-model',
      'reasoning-v1',
      'reasoning-v1',
      'critic-model',
      'reasoning-v1',
      'reasoning-v1',
    ]);
    expect(mockSynthesizeCouncil).toHaveBeenCalledWith({
      question: expect.stringContaining('shared_reasoning.md'),
      members: expect.any(Array),
      chair_model: 'reasoning-v1',
    });
    expect(mockAnswerMember.mock.calls[0][0].question).toContain('User question:');
    expect(mockAnswerMember.mock.calls[0][0].question).toContain('What is the capital of France?');
    expect(mockAnswerMember.mock.calls[0][0].question).toContain('Debate round 1 of 3.');
    expect(mockAnswerMember.mock.calls[8][0].question).toContain('Debate round 3 of 3.');
  });

  it('lets the judge agent use a saved profile unless a model override is typed', async () => {
    mockProgressiveSuccess();
    await renderEditCouncil();

    fireEvent.change(screen.getByLabelText('Judge agent'), { target: { value: 'profile' } });
    fireEvent.change(screen.getByLabelText('Judge profile'), { target: { value: 'critic' } });
    await saveCouncilSettings();
    fillQuestion();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });

    expect(mockAnswerMember.mock.calls.map(call => call[0].model)).toEqual([
      'reasoning-v1',
      'reasoning-v1',
      'reasoning-v1',
      'reasoning-v1',
      'reasoning-v1',
      'reasoning-v1',
      'reasoning-v1',
      'reasoning-v1',
      'reasoning-v1',
    ]);
    expect(mockSynthesizeCouncil).toHaveBeenCalledWith({
      question: expect.any(String),
      members: expect.any(Array),
      chair_model: 'critic-model',
    });
  });

  it('renders member answers side-by-side + the synthesis', async () => {
    mockProgressiveSuccess(RESULT.members);
    await renderOpenCouncil();
    fillQuestion();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });

    await waitFor(() => {
      expect(screen.getByText('Council results')).toBeInTheDocument();
    });
    expect(screen.getByText('Paris is the capital.')).toBeInTheDocument();
    expect(screen.getByText('rate limited')).toBeInTheDocument();
    expect(screen.getByText('Answered')).toBeInTheDocument();
    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(
      screen.getByText('Both that answered agree: Paris. One seat failed.')
    ).toBeInTheDocument();
    expect(screen.getByText('by claude-opus-4-8')).toBeInTheDocument();
    expect(screen.getByText('Debate usage')).toBeInTheDocument();
    expect(screen.getByText('Total')).toBeInTheDocument();
  });

  it('renders council markdown instead of showing raw markdown markers', async () => {
    mockProgressiveSuccess([
      { model: 'reasoning-v1', response: '**Paris** is the capital.', error: null },
      { model: 'reasoning-v1', response: '- France\n- Paris', error: null },
      { model: 'reasoning-v1', response: '`Paris` remains the answer.', error: null },
    ]);
    mockSynthesizeCouncil.mockResolvedValueOnce({
      ...RESULT,
      members: [
        { model: 'reasoning-v1', response: '**Paris** is the capital.', error: null },
        { model: 'reasoning-v1', response: '- France\n- Paris', error: null },
      ],
      synthesis: '## Consensus\n\nThe answer is **Paris**.',
    });
    await renderOpenCouncil();
    fillQuestion();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Consensus' })).toBeInTheDocument();
    });
    const results = screen.getByText('Council results').closest('section');
    expect(results).not.toBeNull();
    expect(screen.getAllByText('Paris').some(node => node.tagName.toLowerCase() === 'strong')).toBe(
      true
    );
    expect(within(results as HTMLElement).queryByText(/\*\*Paris\*\*/)).not.toBeInTheDocument();
  });

  it('surfaces an error alert when the council run fails', async () => {
    mockAnswerMember.mockResolvedValue({
      model: 'reasoning-v1',
      response: null,
      error: 'downstream',
    });
    mockSynthesizeCouncil.mockRejectedValueOnce(new Error('all member models failed to respond'));
    await renderOpenCouncil();
    fillQuestion();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Convene council' }));
    });

    await waitFor(() => {
      const alert = screen.getByRole('alert');
      expect(alert.textContent).toMatch(/all member models failed to respond/);
    });
    expect(screen.queryByText('Council results')).not.toBeInTheDocument();
  });
});
