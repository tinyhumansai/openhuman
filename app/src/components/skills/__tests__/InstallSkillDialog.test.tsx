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
 * - Error panel renders verbatim on failure and submit re-enables.
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
      target: { value: 'http://example.com/pkg.tgz' },
    });
    expect(submit.disabled).toBe(true);
    expect(screen.getByText(/must be a well-formed/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://example.com/pkg.tgz' },
    });
    expect(submit.disabled).toBe(false);
  });

  it('rejects out-of-range timeout values', () => {
    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://example.com/pkg.tgz' },
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
      url: 'https://example.com/pkg.tgz',
      stdout: 'added my-skill',
      stderr: '',
      newSkills: ['my-skill'],
    });

    const onInstalled = vi.fn();
    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={onInstalled} />);

    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://example.com/pkg.tgz' },
    });
    fireEvent.change(screen.getByLabelText(/Timeout/), { target: { value: '120' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Install/ }));
    });

    expect(vi.mocked(skillsApi.installSkillFromUrl)).toHaveBeenCalledWith({
      url: 'https://example.com/pkg.tgz',
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
      url: 'https://example.com/pkg.tgz',
      stdout: '',
      stderr: '',
      newSkills: [],
    });

    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://example.com/pkg.tgz' },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /Install/ }));
    });

    expect(vi.mocked(skillsApi.installSkillFromUrl)).toHaveBeenCalledWith({
      url: 'https://example.com/pkg.tgz',
    });
  });

  it('surfaces error verbatim and re-enables submit', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.installSkillFromUrl).mockRejectedValueOnce(
      new Error('host blocked: localhost')
    );

    render(<InstallSkillDialog onClose={vi.fn()} onInstalled={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Skill URL/), {
      target: { value: 'https://localhost/pkg.tgz' },
    });

    const submit = screen.getByRole('button', { name: /Install/ }) as HTMLButtonElement;
    await act(async () => {
      fireEvent.click(submit);
    });

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('host blocked: localhost');
    });
    expect(submit.disabled).toBe(false);
  });
});
