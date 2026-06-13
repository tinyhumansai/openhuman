/**
 * Tests for the migration panel (#1440).
 *
 * Pins the dry-run-then-apply contract the issue calls out:
 *   - Apply is disabled until a successful Preview lands.
 *   - Apply requires an explicit window.confirm.
 *   - Hermes vendor is selectable and routes to the correct RPC.
 *   - RPC errors render inline without nuking the form state.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import {
  type MigrationReport,
  openhumanMigrateHermes,
  openhumanMigrateOpenclaw,
} from '../../../../utils/tauriCommands/core';
import MigrationPanel from '../MigrationPanel';

vi.mock('../../../../utils/tauriCommands/core', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/tauriCommands/core')>(
    '../../../../utils/tauriCommands/core'
  );
  return { ...actual, openhumanMigrateOpenclaw: vi.fn(), openhumanMigrateHermes: vi.fn() };
});

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

function makeReport(
  overrides: Partial<MigrationReport> = {},
  statsOverrides: Partial<MigrationReport['stats']> = {}
): MigrationReport {
  return {
    source_workspace: '/home/u/.openclaw/workspace',
    target_workspace: '/home/u/.openhuman/workspace',
    dry_run: true,
    stats: {
      from_sqlite: 4,
      from_markdown: 2,
      imported: 0,
      skipped_unchanged: 0,
      renamed_conflicts: 0,
      ...statsOverrides,
    },
    warnings: ['No conflicts detected'],
    ...overrides,
  };
}

describe('MigrationPanel (#1440)', () => {
  beforeEach(() => {
    vi.mocked(openhumanMigrateOpenclaw).mockReset();
    vi.mocked(openhumanMigrateHermes).mockReset();
  });

  it('renders OpenClaw as the default vendor and Hermes as selectable', () => {
    renderWithProviders(<MigrationPanel />);
    const select = screen.getByTestId('migration-vendor-select') as HTMLSelectElement;
    expect(select.value).toBe('openclaw');
    const hermesOption = Array.from(select.options).find(o => o.value === 'hermes');
    expect(hermesOption).toBeDefined();
    expect(hermesOption?.disabled).toBe(false);
    expect(screen.getByTestId('migration-apply-button')).toBeDisabled();
  });

  it('selecting Hermes and clicking Preview calls the Hermes RPC', async () => {
    vi.mocked(openhumanMigrateHermes).mockResolvedValueOnce({
      result: makeReport(
        { source_workspace: '/home/u/.hermes' },
        { from_sqlite: 0, from_markdown: 1 }
      ),
      logs: [],
    });

    renderWithProviders(<MigrationPanel />);
    const select = screen.getByTestId('migration-vendor-select') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'hermes' } });
    fireEvent.click(screen.getByTestId('migration-preview-button'));

    await waitFor(() => expect(screen.getByTestId('migration-report-preview')).toBeInTheDocument());
    expect(openhumanMigrateHermes).toHaveBeenCalledWith(undefined, true);
    expect(openhumanMigrateOpenclaw).not.toHaveBeenCalled();
  });

  it('calls the RPC with dry_run=true on Preview and renders the report', async () => {
    vi.mocked(openhumanMigrateOpenclaw).mockResolvedValueOnce({
      result: makeReport({}, { from_sqlite: 7, from_markdown: 3 }),
      logs: [],
    });

    renderWithProviders(<MigrationPanel />);
    fireEvent.click(screen.getByTestId('migration-preview-button'));

    await waitFor(() => expect(screen.getByTestId('migration-report-preview')).toBeInTheDocument());
    expect(openhumanMigrateOpenclaw).toHaveBeenCalledWith(undefined, true);
    expect(screen.getByTestId('migration-report-source').textContent).toContain(
      '/home/u/.openclaw/workspace'
    );
    expect(screen.getByTestId('migration-report-warnings').textContent).toContain(
      'No conflicts detected'
    );
    // Apply is unlocked now that we have a preview.
    expect(screen.getByTestId('migration-apply-button')).not.toBeDisabled();
  });

  it('passes the user-supplied source path through to the RPC', async () => {
    vi.mocked(openhumanMigrateOpenclaw).mockResolvedValueOnce({ result: makeReport(), logs: [] });
    renderWithProviders(<MigrationPanel />);
    const input = screen.getByTestId('migration-source-input') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '  /opt/legacy/openclaw  ' } });
    fireEvent.click(screen.getByTestId('migration-preview-button'));
    await waitFor(() =>
      expect(openhumanMigrateOpenclaw).toHaveBeenCalledWith('/opt/legacy/openclaw', true)
    );
  });

  it('requires window.confirm before Apply and calls the RPC with dry_run=false on yes', async () => {
    vi.mocked(openhumanMigrateOpenclaw)
      .mockResolvedValueOnce({ result: makeReport(), logs: [] })
      .mockResolvedValueOnce({
        result: makeReport({ dry_run: false }, { from_sqlite: 4, from_markdown: 2, imported: 6 }),
        logs: [],
      });

    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValueOnce(true);
    renderWithProviders(<MigrationPanel />);
    fireEvent.click(screen.getByTestId('migration-preview-button'));
    await waitFor(() => expect(screen.getByTestId('migration-apply-button')).not.toBeDisabled());

    fireEvent.click(screen.getByTestId('migration-apply-button'));
    await waitFor(() =>
      expect(openhumanMigrateOpenclaw).toHaveBeenNthCalledWith(2, undefined, false)
    );
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByTestId('migration-report-applied')).toBeInTheDocument());
    confirmSpy.mockRestore();
  });

  it('skips Apply when the user cancels the confirm dialog', async () => {
    vi.mocked(openhumanMigrateOpenclaw).mockResolvedValueOnce({ result: makeReport(), logs: [] });
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValueOnce(false);
    renderWithProviders(<MigrationPanel />);
    fireEvent.click(screen.getByTestId('migration-preview-button'));
    await waitFor(() => expect(screen.getByTestId('migration-apply-button')).not.toBeDisabled());

    fireEvent.click(screen.getByTestId('migration-apply-button'));
    // Only the Preview call should have fired — Apply must not call the RPC
    // when the operator says no.
    expect(openhumanMigrateOpenclaw).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('migration-report-applied')).toBeNull();
    confirmSpy.mockRestore();
  });

  it('re-disables Apply when the source path is edited after a preview', async () => {
    // CodeRabbit regression guard on PR #2087: previously the Apply button
    // stayed unlocked after any prior preview, so editing the source path
    // could run Apply against a workspace that was never previewed.
    vi.mocked(openhumanMigrateOpenclaw).mockResolvedValueOnce({ result: makeReport(), logs: [] });
    renderWithProviders(<MigrationPanel />);
    const input = screen.getByTestId('migration-source-input') as HTMLInputElement;
    const apply = screen.getByTestId('migration-apply-button');

    // Preview with no source path → Apply unlocks.
    fireEvent.click(screen.getByTestId('migration-preview-button'));
    await waitFor(() => expect(apply).not.toBeDisabled());

    // User edits the path after previewing — Apply must re-lock until
    // they preview the new value.
    fireEvent.change(input, { target: { value: '/opt/legacy/openclaw' } });
    expect(apply).toBeDisabled();
  });

  it('re-disables Apply when vendor is switched after a preview', async () => {
    vi.mocked(openhumanMigrateOpenclaw).mockResolvedValueOnce({ result: makeReport(), logs: [] });
    renderWithProviders(<MigrationPanel />);
    const apply = screen.getByTestId('migration-apply-button');

    fireEvent.click(screen.getByTestId('migration-preview-button'));
    await waitFor(() => expect(apply).not.toBeDisabled());

    const select = screen.getByTestId('migration-vendor-select') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'hermes' } });
    expect(apply).toBeDisabled();
  });

  it('renders inline error on RPC failure without removing the form', async () => {
    vi.mocked(openhumanMigrateOpenclaw).mockRejectedValueOnce(
      new Error('OpenClaw workspace not found at /opt/legacy/openclaw')
    );
    renderWithProviders(<MigrationPanel />);
    fireEvent.click(screen.getByTestId('migration-preview-button'));
    await waitFor(() => expect(screen.getByTestId('migration-error')).toBeInTheDocument());
    expect(screen.getByTestId('migration-error').textContent).toContain(
      'OpenClaw workspace not found'
    );
    // Form survives so the user can edit + retry.
    expect(screen.getByTestId('migration-form')).toBeInTheDocument();
    expect(screen.getByTestId('migration-apply-button')).toBeDisabled();
  });
});
