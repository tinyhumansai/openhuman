/**
 * Small render test for the settings-panel disclosure in `MemorySourceRow`
 * (#radix-ui-foundation). The gear button is a `CollapsibleTrigger` wrapping
 * a `Button` via `asChild`, and the panel is a `CollapsibleContent` — Radix
 * unmounts it from the DOM while collapsed, which is a genuine behavior
 * change from the old `{settingsExpanded && <SourceSettingsPanel />}` (which
 * only ever rendered it conditionally too, so the unmount contract is
 * unchanged in practice; this test pins that both states still work).
 *
 * Full interaction coverage (per-kind fields, Save, etc.) lives in
 * `__tests__/MemorySourcesRegistry.test.tsx`, which renders this row through
 * its real parent.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { MemorySourceEntry } from '../../services/memorySourcesService';
import { MemorySourceRow } from './MemorySourceRow';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function makeSource(overrides: Partial<MemorySourceEntry> = {}): MemorySourceEntry {
  return {
    id: 'src_1',
    kind: 'github_repo',
    label: 'My Repo',
    enabled: true,
    url: 'https://github.com/org/repo',
    ...overrides,
  };
}

function renderRow(overrides: Partial<React.ComponentProps<typeof MemorySourceRow>> = {}) {
  const onToggleSettings = vi.fn();
  const props: React.ComponentProps<typeof MemorySourceRow> = {
    source: makeSource(),
    status: null,
    pipeline: null,
    backfill: null,
    isAuthenticated: true,
    isSyncing: false,
    isBuilding: false,
    progress: null,
    result: null,
    settingsExpanded: false,
    onToggle: vi.fn(),
    onRemove: vi.fn(),
    onSync: vi.fn(),
    onBuild: vi.fn(),
    onToggleSettings,
    onSettingsSaved: vi.fn(),
    onViewHealth: vi.fn(),
    onSignIn: vi.fn(),
    ...overrides,
  };
  render(
    <ul>
      <MemorySourceRow {...props} />
    </ul>
  );
  return { onToggleSettings };
}

describe('MemorySourceRow — settings disclosure', () => {
  it('keeps the settings panel unmounted while collapsed', () => {
    renderRow({ settingsExpanded: false });
    expect(screen.queryByTestId('source-settings-panel-src_1')).not.toBeInTheDocument();
    expect(screen.getByTestId('memory-source-settings-src_1')).toHaveAttribute(
      'aria-expanded',
      'false'
    );
  });

  it('renders the settings panel when the parent marks it expanded', () => {
    renderRow({ settingsExpanded: true });
    expect(screen.getByTestId('source-settings-panel-src_1')).toBeInTheDocument();
    expect(screen.getByTestId('memory-source-settings-src_1')).toHaveAttribute(
      'aria-expanded',
      'true'
    );
  });

  it('delegates the toggle to onToggleSettings rather than owning its own open state', () => {
    const { onToggleSettings } = renderRow({ settingsExpanded: false });
    fireEvent.click(screen.getByTestId('memory-source-settings-src_1'));
    expect(onToggleSettings).toHaveBeenCalledWith('src_1');
  });
});
