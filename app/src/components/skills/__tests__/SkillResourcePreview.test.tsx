/**
 * SkillResourcePreview — vitest coverage
 *
 * Verifies:
 * - Loading state renders a spinner.
 * - Success path renders `content` in a <pre> and shows the size footer.
 * - Error path surfaces the backend error string verbatim (e.g. "path escape").
 * - Close button triggers onDismiss.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { skillsApi } from '../../../services/api/skillsApi';
import SkillResourcePreview from '../SkillResourcePreview';

vi.mock('../../../services/api/skillsApi', () => ({
  skillsApi: {
    listSkills: vi.fn(),
    readSkillResource: vi.fn(),
  },
}));

const mockedReadSkillResource = skillsApi.readSkillResource as ReturnType<typeof vi.fn>;

describe('SkillResourcePreview', () => {
  beforeEach(() => {
    mockedReadSkillResource.mockReset();
  });

  it('renders a loading spinner while the RPC is pending', () => {
    mockedReadSkillResource.mockImplementation(() => new Promise(() => {}));
    render(
      <SkillResourcePreview skillId="demo" relativePath="scripts/run.sh" onDismiss={vi.fn()} />
    );
    expect(screen.getByRole('status', { name: /loading/i })).toBeInTheDocument();
  });

  it('renders content and a size footer on success', async () => {
    mockedReadSkillResource.mockResolvedValueOnce({
      skillId: 'demo',
      relativePath: 'scripts/run.sh',
      content: '#!/bin/sh\necho hello',
      bytes: 20,
    });
    render(
      <SkillResourcePreview skillId="demo" relativePath="scripts/run.sh" onDismiss={vi.fn()} />
    );
    await waitFor(() => {
      expect(screen.getByText('#!/bin/sh echo hello')).toBeInTheDocument();
    });
    // Size footer ("20 B")
    expect(screen.getByText('20 B')).toBeInTheDocument();
  });

  it('surfaces the backend error string verbatim on failure', async () => {
    mockedReadSkillResource.mockRejectedValueOnce(new Error('path escape'));
    render(
      <SkillResourcePreview
        skillId="demo"
        relativePath="../../etc/passwd"
        onDismiss={vi.fn()}
      />
    );
    await waitFor(() => {
      expect(screen.getByText('Preview failed')).toBeInTheDocument();
    });
    expect(screen.getByText('path escape')).toBeInTheDocument();
  });

  it('surfaces size-cap errors verbatim', async () => {
    mockedReadSkillResource.mockRejectedValueOnce(
      new Error('resource exceeds maximum size of 131072 bytes')
    );
    render(
      <SkillResourcePreview skillId="demo" relativePath="assets/huge.bin" onDismiss={vi.fn()} />
    );
    await waitFor(() => {
      expect(
        screen.getByText(/resource exceeds maximum size of 131072 bytes/i)
      ).toBeInTheDocument();
    });
  });

  it('calls onDismiss when the close button is clicked', async () => {
    mockedReadSkillResource.mockResolvedValueOnce({
      skillId: 'demo',
      relativePath: 'scripts/run.sh',
      content: 'ok',
      bytes: 2,
    });
    const onDismiss = vi.fn();
    render(
      <SkillResourcePreview
        skillId="demo"
        relativePath="scripts/run.sh"
        onDismiss={onDismiss}
      />
    );
    await act(async () => {
      // allow promise to settle
      await Promise.resolve();
    });
    fireEvent.click(screen.getByRole('button', { name: /close preview/i }));
    expect(onDismiss).toHaveBeenCalled();
  });
});
