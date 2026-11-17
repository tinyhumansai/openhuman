import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ScanBlocked } from '../../../services/api/skillRegistryApi';
import ScanBlockedDialog from '../ScanBlockedDialog';

const scan: ScanBlocked = {
  target: 'poisoned',
  fetchedFrom: 'https://example.com/SKILL.md',
  slug: 'poisoned',
  digest: 'd1',
  findings: [
    {
      check: 'invisible_code_points',
      verdict: 'block',
      field: 'the document body',
      message: 'an invisible character (U+200B) in the document body',
    },
    {
      check: 'instruction_shaped',
      verdict: 'warn',
      field: 'description',
      message: 'text addressed to the agent in description',
    },
  ],
  message: 'blocked',
};

function renderDialog(overrides: Partial<Parameters<typeof ScanBlockedDialog>[0]> = {}) {
  const onBlock = vi.fn();
  const onInstallAnyway = vi.fn();
  render(
    <ScanBlockedDialog
      skillName="Poisoned"
      scan={scan}
      installing={false}
      onBlock={onBlock}
      onInstallAnyway={onInstallAnyway}
      {...overrides}
    />
  );
  return { onBlock, onInstallAnyway };
}

describe('ScanBlockedDialog', () => {
  it('lists every finding with its verdict and names the skill', () => {
    renderDialog();
    expect(screen.getByText('Security scan blocked this skill')).toBeInTheDocument();
    expect(screen.getByText(/scanned Poisoned twice/)).toBeInTheDocument();
    expect(screen.getByText(/invisible character \(U\+200B\)/)).toBeInTheDocument();
    expect(screen.getByText(/text addressed to the agent/)).toBeInTheDocument();
    expect(screen.getByText('Blocked')).toBeInTheDocument();
    expect(screen.getByText('Warning')).toBeInTheDocument();
  });

  it('focuses Block install by default', () => {
    renderDialog();
    expect(screen.getByTestId('scan-blocked-block')).toHaveFocus();
    expect(screen.getByTestId('scan-blocked-block')).toHaveTextContent('Block install');
  });

  it('Block install and Escape both keep the skill uninstalled', () => {
    const { onBlock, onInstallAnyway } = renderDialog();
    fireEvent.click(screen.getByTestId('scan-blocked-block'));
    expect(onBlock).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(screen.getByTestId('scan-blocked-dialog'), { key: 'Escape' });
    expect(onBlock).toHaveBeenCalledTimes(2);
    expect(onInstallAnyway).not.toHaveBeenCalled();
  });

  it('Install anyway asks the caller to install', () => {
    const { onBlock, onInstallAnyway } = renderDialog();
    fireEvent.click(screen.getByTestId('scan-blocked-install-anyway'));
    expect(onInstallAnyway).toHaveBeenCalledTimes(1);
    expect(onBlock).not.toHaveBeenCalled();
  });

  it('locks both choices while installing and shows an install error', () => {
    const { onBlock } = renderDialog({ installing: true, error: 'write failed: disk full' });
    expect(screen.getByTestId('scan-blocked-block')).toBeDisabled();
    expect(screen.getByTestId('scan-blocked-install-anyway')).toBeDisabled();
    expect(screen.getByRole('alert')).toHaveTextContent('write failed: disk full');
    fireEvent.keyDown(screen.getByTestId('scan-blocked-dialog'), { key: 'Escape' });
    expect(onBlock).not.toHaveBeenCalled();
  });
});
