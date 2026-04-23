/**
 * InstallSkillDialog — vitest coverage
 *
 * Verifies:
 * - Renders title + url input + install button.
 * - Submit disabled until a well-formed https URL is entered.
 * - Shows inline error for non-https URLs.
 * - Rejects timeout outside 1–600.
 * - Submit forwards timeoutSecs to skillsApi.installSkillFromUrl.
 * - Success panel renders newSkills list + calls onInstalled.
 * - Error panel categorizes known prefixes and shows the raw error in
 *   a details expander; unknown errors fall back to a generic title.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import InstallSkillDialog from '../InstallSkillDialog';

vi.mock('../../../services/api/skillsApi', () => ({
  skillsApi: {
    installSkillFromUrl: vi.fn(),
  },
}));

describe('InstallSkillDialog', () => {
  beforeEach(async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.installSkillFromUrl).mockReset();
  });

  it('renders title and URL input', () => {
    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    expect(screen.getByText('Install skill from URL')).toBeInTheDocument();
    expect(screen.getByLabelText(/Skill URL/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Install/ })).toBeInTheDocument();
  });

  it('disables submit until a well-formed https URL is entered', () => {
    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    const submit = screen.getByRole('button', { name: /Install/ }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText(/Skill URL/), { target: { value: 'not-a-url' } });
    expect(submit.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'http://example.com/SKILL.md' },
    });
    expect(submit.disabled).toBe(true);
    expect(screen.getByText(/must be a well-formed/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://raw.githubusercontent.com/owner/repo/main/SKILL.md' },
    });
    expect(submit.disabled).toBe(false);
  });

  it('rejects out-of-range timeout values', () => {
    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://raw.githubusercontent.com/owner/repo/main/SKILL.md' },
    });

    const submit = screen.getByRole('button', { name: /Install/ }) as HTMLButtonElement;
    expect(submit.disabled).toBe(false);

    fireEvent.change(screen.getByLabelText(/Timeout/), { target: { value: '9999' } });
    expect(submit.disabled).toBe(true);
    expect(screen.getByText(/Must be an integer between 1 and 600/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/Timeout/), { target: { value: '120' } });
    expect(submit.disabled).toBe(false);
  });

  it('forwards timeoutSecs to skillsApi and fires onInstalled on success', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.installSkillFromUrl).mockResolvedValueOnce({
      url: 'https://raw.githubusercontent.com/owner/repo/main/SKILL.md',
      stdout: 'added my-skill',
      stderr: '',
      newSkills: ['my-skill'],
    });

    const onInstalled = vi.fn();
    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={onInstalled} />);

    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://raw.githubusercontent.com/owner/repo/main/SKILL.md' },
    });
    fireEvent.change(screen.getByLabelText(/Timeout/), { target: { value: '120' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Install/ }));
    });

    expect(vi.mocked(skillsApi.installSkillFromUrl)).toHaveBeenCalledWith({
      url: 'https://raw.githubusercontent.com/owner/repo/main/SKILL.md',
      timeoutSecs: 120,
    });
    await waitFor(() => {
      expect(screen.getByText('Install complete')).toBeInTheDocument();
    });
    expect(screen.getByText('my-skill')).toBeInTheDocument();
    expect(onInstalled).toHaveBeenCalledWith(
      expect.objectContaining({ newSkills: ['my-skill'] })
    );
  });

  it('omits timeoutSecs when field is blank', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.installSkillFromUrl).mockResolvedValueOnce({
      url: 'https://raw.githubusercontent.com/owner/repo/main/SKILL.md',
      stdout: '',
      stderr: '',
      newSkills: [],
    });

    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://raw.githubusercontent.com/owner/repo/main/SKILL.md' },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Install/ }));
    });

    expect(vi.mocked(skillsApi.installSkillFromUrl)).toHaveBeenCalledWith({
      url: 'https://raw.githubusercontent.com/owner/repo/main/SKILL.md',
    });
  });

  it('shows generic title with raw error text on unknown error and re-enables submit', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.installSkillFromUrl).mockRejectedValueOnce(
      new Error('unexpected: something weird happened')
    );

    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://example.com/SKILL.md' },
    });

    const submit = screen.getByRole('button', { name: /Install/ }) as HTMLButtonElement;
    await act(async () => {
      fireEvent.click(submit);
    });

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('Could not install skill');
    });
    expect(screen.getByRole('alert')).toHaveTextContent(
      'unexpected: something weird happened'
    );
    expect(submit.disabled).toBe(false);
  });

  it('categorizes "invalid SKILL.md:" errors with a friendly title and hint', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.installSkillFromUrl).mockRejectedValueOnce(
      new Error('invalid SKILL.md: missing required field `description`')
    );

    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://example.com/SKILL.md' },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Install/ }));
    });

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('SKILL.md did not parse');
    });
    expect(screen.getByRole('alert')).toHaveTextContent(/frontmatter must be valid YAML/i);
    expect(screen.getByRole('alert')).toHaveTextContent('missing required field');
  });

  it('categorizes "unsupported url form:" errors', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.installSkillFromUrl).mockRejectedValueOnce(
      new Error('unsupported url form: path must end in .md, got "https://example.com/foo"')
    );

    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://example.com/SKILL.md' },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Install/ }));
    });

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('URL form not supported');
    });
    expect(screen.getByRole('alert')).toHaveTextContent(/direct `?\.md`? links/i);
  });
});
