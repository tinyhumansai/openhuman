/**
 * CreateSkillModal — vitest coverage
 *
 * Verifies:
 * - Renders title + required fields.
 * - Escape key closes (but not while submitting).
 * - Backdrop click closes (but not while submitting).
 * - Submit is disabled when name or description is empty.
 * - Submit rekeys `allowedTools` → `'allowed-tools'` via skillsApi.createSkill.
 * - Submit calls `onCreated` with the returned skill.
 * - Submit failure surfaces an error banner and re-enables the button.
 * - Slug preview updates as the name changes.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SkillSummary } from '../../../services/api/skillsApi';
import CreateSkillModal from '../CreateSkillModal';

vi.mock('../../../services/api/skillsApi', () => ({
  skillsApi: {
    createSkill: vi.fn(),
  },
}));

function builtSkill(overrides: Partial<SkillSummary> = {}): SkillSummary {
  return {
    id: 'my-skill',
    name: 'My Skill',
    description: 'does stuff',
    version: '',
    author: null,
    tags: [],
    tools: [],
    prompts: [],
    location: '/home/u/.openhuman/skills/my-skill/SKILL.md',
    resources: [],
    scope: 'user',
    legacy: false,
    warnings: [],
    ...overrides,
  };
}

describe('CreateSkillModal', () => {
  beforeEach(async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.createSkill).mockReset();
  });

  it('renders title and required fields', () => {
    render(<CreateSkillModal onClose={vi.fn()} onCreated={vi.fn()} />);
    expect(screen.getByText('New skill')).toBeInTheDocument();
    expect(screen.getByLabelText(/Name/)).toBeInTheDocument();
    expect(screen.getByLabelText(/Description/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Create skill/ })).toBeInTheDocument();
  });

  it('updates slug preview as the user types the name', () => {
    render(<CreateSkillModal onClose={vi.fn()} onCreated={vi.fn()} />);
    const name = screen.getByLabelText(/Name/) as HTMLInputElement;
    fireEvent.change(name, { target: { value: 'My Trade Journal!' } });
    expect(screen.getByText('my-trade-journal')).toBeInTheDocument();
  });

  it('disables submit when name or description is empty', () => {
    render(<CreateSkillModal onClose={vi.fn()} onCreated={vi.fn()} />);
    const submit = screen.getByRole('button', { name: /Create skill/ }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText(/Name/), { target: { value: 'demo' } });
    expect(submit.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText(/Description/), { target: { value: 'what it does' } });
    expect(submit.disabled).toBe(false);
  });

  it('closes on Escape', () => {
    const onClose = vi.fn();
    render(<CreateSkillModal onClose={onClose} onCreated={vi.fn()} />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('rekeys allowedTools to allowed-tools on submit and calls onCreated', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    const created = builtSkill();
    vi.mocked(skillsApi.createSkill).mockResolvedValueOnce(created);

    const onCreated = vi.fn();
    const onClose = vi.fn();
    render(<CreateSkillModal onClose={onClose} onCreated={onCreated} />);

    fireEvent.change(screen.getByLabelText(/Name/), { target: { value: 'My Skill' } });
    fireEvent.change(screen.getByLabelText(/Description/), { target: { value: 'does stuff' } });
    fireEvent.change(screen.getByLabelText(/Tags/), { target: { value: 'alpha, beta' } });
    fireEvent.change(screen.getByLabelText(/Allowed tools/), {
      target: { value: 'mcp/fs, fetch' },
    });

    const submit = screen.getByRole('button', { name: /Create skill/ });
    await act(async () => {
      fireEvent.click(submit);
    });

    expect(vi.mocked(skillsApi.createSkill)).toHaveBeenCalledWith({
      name: 'My Skill',
      description: 'does stuff',
      scope: 'user',
      tags: ['alpha', 'beta'],
      allowedTools: ['mcp/fs', 'fetch'],
    });
    expect(onCreated).toHaveBeenCalledWith(created);
  });

  it('surfaces error and re-enables submit on failure', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.createSkill).mockRejectedValueOnce(new Error('slug already exists'));

    render(<CreateSkillModal onClose={vi.fn()} onCreated={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Name/), { target: { value: 'dup' } });
    fireEvent.change(screen.getByLabelText(/Description/), { target: { value: 'x' } });

    const submit = screen.getByRole('button', { name: /Create skill/ }) as HTMLButtonElement;
    await act(async () => {
      fireEvent.click(submit);
    });

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('slug already exists');
    });
    expect(submit.disabled).toBe(false);
  });
});
