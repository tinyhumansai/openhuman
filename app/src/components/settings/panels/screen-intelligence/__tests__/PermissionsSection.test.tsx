/**
 * Tests for the Screen Awareness → Permissions section.
 *
 * PermissionsSection is a pure, props-driven presentational component (no
 * hooks/RPC of its own beyond i18n). These tests exercise both the
 * "permission denied" branch (denied alert + macOS process-path hint +
 * Restart & Refresh button) and the "all granted" branch (plain Refresh
 * Status button), plus the per-permission request buttons and the
 * granted/denied/unknown badge variants.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import PermissionsSection from '../PermissionsSection';

type Props = React.ComponentProps<typeof PermissionsSection>;

const baseProps = (): Props => ({
  screenRecording: 'granted',
  accessibility: 'denied',
  inputMonitoring: 'unknown',
  anyPermissionDenied: true,
  lastRestartSummary: null,
  permissionCheckProcessPath: null,
  isRequestingPermissions: false,
  isRestartingCore: false,
  isLoading: false,
  requestPermission: vi.fn().mockResolvedValue(null),
  refreshPermissionsWithRestart: vi.fn().mockResolvedValue(null),
  refreshStatus: vi.fn().mockResolvedValue(null),
});

describe('PermissionsSection', () => {
  it('renders denied alert with process path, restart summary, and request buttons', () => {
    const props = {
      ...baseProps(),
      anyPermissionDenied: true,
      permissionCheckProcessPath: '/tmp/openhuman-core',
      lastRestartSummary: 'Core restarted: PID 4000 -> PID 4242.',
    };
    render(<PermissionsSection {...props} />);

    // Mixed badge variants render (granted/denied/unknown).
    expect(screen.getByText('granted')).toBeInTheDocument();
    expect(screen.getByText('denied')).toBeInTheDocument();
    expect(screen.getByText('unknown')).toBeInTheDocument();

    // Denied-alert block surfaces the macOS process path (line ~69).
    expect(screen.getByText('/tmp/openhuman-core')).toBeInTheDocument();

    // Last restart summary is shown.
    expect(screen.getByText('Core restarted: PID 4000 -> PID 4242.')).toBeInTheDocument();

    // The three per-permission request buttons render.
    expect(screen.getByRole('button', { name: 'Request Screen Recording' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Request Accessibility' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open Input Monitoring' })).toBeInTheDocument();
  });

  it('invokes the request handlers and the restart-with-refresh handler when denied', () => {
    const props = { ...baseProps(), anyPermissionDenied: true };
    render(<PermissionsSection {...props} />);

    fireEvent.click(screen.getByRole('button', { name: 'Request Screen Recording' }));
    expect(props.requestPermission).toHaveBeenCalledWith('screen_recording');

    fireEvent.click(screen.getByRole('button', { name: 'Request Accessibility' }));
    expect(props.requestPermission).toHaveBeenCalledWith('accessibility');

    fireEvent.click(screen.getByRole('button', { name: 'Open Input Monitoring' }));
    expect(props.requestPermission).toHaveBeenCalledWith('input_monitoring');

    // When permissions are denied, the danger Restart & Refresh button shows.
    const restartBtn = screen.getByRole('button', { name: 'Restarting core…' });
    fireEvent.click(restartBtn);
    expect(props.refreshPermissionsWithRestart).toHaveBeenCalledTimes(1);
    expect(props.refreshStatus).not.toHaveBeenCalled();
  });

  it('shows the plain Refresh Status button when nothing is denied', () => {
    const props = {
      ...baseProps(),
      screenRecording: 'granted',
      accessibility: 'granted',
      inputMonitoring: 'granted',
      anyPermissionDenied: false,
      permissionCheckProcessPath: null,
    };
    render(<PermissionsSection {...props} />);

    // The denied-alert process path is absent when nothing is denied.
    expect(screen.queryByText('/tmp/openhuman-core')).not.toBeInTheDocument();

    // The restart-with-refresh button is replaced by the plain refresh button.
    expect(screen.queryByRole('button', { name: 'Restarting core…' })).not.toBeInTheDocument();

    const refreshBtn = screen.getByRole('button', { name: 'Refreshing…' });
    fireEvent.click(refreshBtn);
    expect(props.refreshStatus).toHaveBeenCalledTimes(1);
    expect(props.refreshPermissionsWithRestart).not.toHaveBeenCalled();
  });

  it('reflects in-flight labels while requesting permissions', () => {
    const props = { ...baseProps(), isRequestingPermissions: true };
    render(<PermissionsSection {...props} />);

    // All three request buttons collapse to the "Requesting…" label.
    expect(screen.getAllByRole('button', { name: 'Requesting…' })).toHaveLength(3);
  });

  it('shows the restarting-core label and disables the restart button while restarting', () => {
    // Exercises the `isRestartingCore ? …` branch of the denied restart button.
    const props = { ...baseProps(), anyPermissionDenied: true, isRestartingCore: true };
    render(<PermissionsSection {...props} />);

    const restartBtn = screen.getByRole('button', { name: 'Restarting core…' });
    expect(restartBtn).toBeDisabled();
    // While restarting, the request buttons are also disabled.
    expect(screen.getByRole('button', { name: 'Request Screen Recording' })).toBeDisabled();
  });

  it('shows the refreshing label and disables the refresh button while loading', () => {
    // Exercises the `isLoading ? …` branch of the not-denied refresh button.
    const props = { ...baseProps(), anyPermissionDenied: false, isLoading: true };
    render(<PermissionsSection {...props} />);

    const refreshBtn = screen.getByRole('button', { name: 'Refreshing…' });
    expect(refreshBtn).toBeDisabled();
  });
});
