/**
 * UninstallSkillConfirmDialog — vitest coverage
 *
 * Verifies:
 * - Renders skill name + on-disk path + destructive confirm copy.
 * - Cancel button fires onClose, does NOT hit the RPC.
 * - Confirm fires `skillsApi.uninstallSkill(name)` and forwards the result
 *   to `onUninstalled`, then closes.
 * - RPC error is surfaced inline and the dialog stays open (no onClose).
 * - While in-flight, both buttons disable and Esc no-ops (handled by
 *   disabled flag on the cancel button; dialog-level dismissal blocked).
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import UninstallSkillConfirmDialog from '../UninstallSkillConfirmDialog';
import type { SkillSummary } from '../../../services/api/skillsApi';

vi.mock('../../../services/api/skillsApi', () => ({
  skillsApi: {
    uninstallSkill: vi.fn(),
  },
}));

const fixture: SkillSummary = {
  id: 'weather-helper',
  name: 'weather-helper',
  description: 'Weather forecasts',
  version: '',
  author: null,
  tags: [],
  tools: [],
  prompts: [],
  location: '/Users/me/.openhuman/skills/weather-helper/SKILL.md',
  resources: [],
  scope: 'user',
  legacy: false,
  warnings: [],
};

describe('UninstallSkillConfirmDialog', () => {
  beforeEach(async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.uninstallSkill).mockReset();
  });

  it('renders skill name, path (stripped of /SKILL.md), and confirm copy', () => {
    render(
      <UninstallSkillConfirmDialog
        skill={fixture}
        onClose={vi.fn()}
        onUninstalled={vi.fn()}
      />
    );
    expect(screen.getByText(/Uninstall weather-helper\?/)).toBeInTheDocument();
    expect(screen.getByText(/permanently deletes/i)).toBeInTheDocument();
    expect(screen.getByText('/Users/me/.openhuman/skills/weather-helper')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Cancel/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^Uninstall$/ })).toBeInTheDocument();
  });

  it('Cancel fires onClose without calling the RPC', async () => {
    const onClose = vi.fn();
    const { skillsApi } = await import('../../../services/api/skillsApi');
    render(
      <UninstallSkillConfirmDialog
        skill={fixture}
        onClose={onClose}
        onUninstalled={vi.fn()}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Cancel/ }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(vi.mocked(skillsApi.uninstallSkill)).not.toHaveBeenCalled();
  });

  it('Confirm calls skillsApi.uninstallSkill and forwards result to onUninstalled', async () => {
    const onClose = vi.fn();
    const onUninstalled = vi.fn();
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.uninstallSkill).mockResolvedValueOnce({
      name: 'weather-helper',
      removedPath: '/Users/me/.openhuman/skills/weather-helper',
      scope: 'user',
    });

    render(
      <UninstallSkillConfirmDialog
        skill={fixture}
        onClose={onClose}
        onUninstalled={onUninstalled}
      />
    );
    fireEvent.click(screen.getByTestId('uninstall-skill-confirm'));

    await waitFor(() => {
      expect(vi.mocked(skillsApi.uninstallSkill)).toHaveBeenCalledWith('weather-helper');
    });
    await waitFor(() => {
      expect(onUninstalled).toHaveBeenCalledWith({
        name: 'weather-helper',
        removedPath: '/Users/me/.openhuman/skills/weather-helper',
        scope: 'user',
      });
    });
    await waitFor(() => {
      expect(onClose).toHaveBeenCalledTimes(1);
    });
  });

  it('surfaces RPC errors inline and keeps the dialog open', async () => {
    const onClose = vi.fn();
    const onUninstalled = vi.fn();
    const { skillsApi } = await import('../../../services/api/skillsApi');
    vi.mocked(skillsApi.uninstallSkill).mockRejectedValueOnce(
      new Error("skill 'weather-helper' is not installed")
    );

    render(
      <UninstallSkillConfirmDialog
        skill={fixture}
        onClose={onClose}
        onUninstalled={onUninstalled}
      />
    );
    fireEvent.click(screen.getByTestId('uninstall-skill-confirm'));

    await waitFor(() => {
      expect(screen.getByText(/Could not uninstall/)).toBeInTheDocument();
    });
    expect(screen.getByText(/is not installed/)).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
    expect(onUninstalled).not.toHaveBeenCalled();
    // Confirm button should be re-enabled so the user can retry.
    const confirm = screen.getByTestId('uninstall-skill-confirm') as HTMLButtonElement;
    expect(confirm.disabled).toBe(false);
  });

  it('disables buttons while the RPC is in flight', async () => {
    const { skillsApi } = await import('../../../services/api/skillsApi');
    type UninstallResolve = (v: {
      name: string;
      removedPath: string;
      scope: SkillSummary['scope'];
    }) => void;
    const deferred: { resolve?: UninstallResolve } = {};
    vi.mocked(skillsApi.uninstallSkill).mockReturnValueOnce(
      new Promise<{
        name: string;
        removedPath: string;
        scope: SkillSummary['scope'];
      }>(resolve => {
        deferred.resolve = resolve;
      })
    );

    render(
      <UninstallSkillConfirmDialog
        skill={fixture}
        onClose={vi.fn()}
        onUninstalled={vi.fn()}
      />
    );
    fireEvent.click(screen.getByTestId('uninstall-skill-confirm'));

    await waitFor(() => {
      const cancel = screen.getByRole('button', { name: /Cancel/ }) as HTMLButtonElement;
      const confirm = screen.getByTestId('uninstall-skill-confirm') as HTMLButtonElement;
      expect(cancel.disabled).toBe(true);
      expect(confirm.disabled).toBe(true);
      expect(confirm.textContent).toMatch(/Uninstalling/);
    });

    deferred.resolve?.({
      name: 'weather-helper',
      removedPath: '/Users/me/.openhuman/skills/weather-helper',
      scope: 'user',
    });
  });
});
