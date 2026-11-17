import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../test/test-utils';
import TimezoneSelect from './TimezoneSelect';

const getMock = vi.fn();
const updateMock = vi.fn();
vi.mock('../utils/tauriCommands', () => ({
  openhumanGetUserTimezone: () => getMock(),
  openhumanUpdateUserTimezone: (zone: string | null) => updateMock(zone),
}));

const settings = (timezone: string | null) => ({
  result: { timezone, device: 'Asia/Kolkata', effective: timezone ?? 'Asia/Kolkata' },
  logs: [],
});

function render() {
  return renderWithProviders(<TimezoneSelect />, { preloadedState: { locale: { current: 'en' } } });
}

describe('TimezoneSelect', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows the saved zone, and offers the device zone by name', async () => {
    getMock.mockResolvedValue(settings('Europe/Berlin'));
    render();
    const select = await screen.findByTestId('timezone-select');
    await waitFor(() => expect((select as HTMLSelectElement).value).toBe('Europe/Berlin'));
    expect(
      screen.getByRole('option', { name: 'Use device time zone (Asia/Kolkata)' })
    ).toBeTruthy();
  });

  it('saves a chosen zone and shows it without a second read', async () => {
    getMock.mockResolvedValue(settings(null));
    updateMock.mockResolvedValue({ result: {}, logs: [] });
    render();
    const select = (await screen.findByTestId('timezone-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.disabled).toBe(false));
    expect(select.value).toBe('');

    fireEvent.change(select, { target: { value: 'America/New_York' } });
    await waitFor(() => expect(updateMock).toHaveBeenCalledWith('America/New_York'));
    await waitFor(() => expect(select.disabled).toBe(false));
    expect(select.value).toBe('America/New_York');
    expect(getMock).toHaveBeenCalledTimes(1);
  });

  it('following the device sends null', async () => {
    getMock.mockResolvedValue(settings('Europe/Berlin'));
    updateMock.mockResolvedValue({ result: {}, logs: [] });
    render();
    const select = (await screen.findByTestId('timezone-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe('Europe/Berlin'));
    fireEvent.change(select, { target: { value: '' } });
    await waitFor(() => expect(updateMock).toHaveBeenCalledWith(null));
  });

  it('a failed save restores the previous zone and says so', async () => {
    getMock.mockResolvedValue(settings('Europe/Berlin'));
    updateMock.mockRejectedValue(new Error('core offline'));
    render();
    const select = (await screen.findByTestId('timezone-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe('Europe/Berlin'));
    fireEvent.change(select, { target: { value: 'Asia/Tokyo' } });
    expect(await screen.findByTestId('timezone-error')).toBeTruthy();
    expect(select.value).toBe('Europe/Berlin');
  });

  it('a saved value that is not an offered zone shows as following the device', async () => {
    getMock.mockResolvedValue(settings('Not/AZone'));
    render();
    const select = (await screen.findByTestId('timezone-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.disabled).toBe(false));
    expect(select.value).toBe('');
    expect(screen.queryByRole('option', { name: 'Not/AZone' })).toBeNull();
  });

  it('shows the old zone until the save succeeds', async () => {
    getMock.mockResolvedValue(settings('Europe/Berlin'));
    let finish: (value: unknown) => void = () => {};
    updateMock.mockReturnValue(new Promise(resolve => (finish = resolve)));
    render();
    const select = (await screen.findByTestId('timezone-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe('Europe/Berlin'));
    fireEvent.change(select, { target: { value: 'Asia/Tokyo' } });
    await waitFor(() => expect(select.disabled).toBe(true));
    expect(select.value).toBe('Europe/Berlin');
    finish({ result: {}, logs: [] });
    await waitFor(() => expect(select.value).toBe('Asia/Tokyo'));
  });

  it('is disabled while a save is in flight, so picks cannot overlap', async () => {
    getMock.mockResolvedValue(settings('Europe/Berlin'));
    let finish: (value: unknown) => void = () => {};
    updateMock.mockReturnValue(new Promise(resolve => (finish = resolve)));
    render();
    const select = (await screen.findByTestId('timezone-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.disabled).toBe(false));
    fireEvent.change(select, { target: { value: 'Asia/Tokyo' } });
    await waitFor(() => expect(select.disabled).toBe(true));
    finish({ result: {}, logs: [] });
    await waitFor(() => expect(select.disabled).toBe(false));
    expect(updateMock).toHaveBeenCalledTimes(1);
  });

  it('a core that cannot be read leaves the picker disabled, not broken', async () => {
    getMock.mockRejectedValue(new Error('core offline'));
    render();
    expect(await screen.findByTestId('timezone-error')).toBeTruthy();
    expect((screen.getByTestId('timezone-select') as HTMLSelectElement).disabled).toBe(true);
  });
  it('offers UTC', async () => {
    getMock.mockResolvedValue(settings(null));
    render();
    await screen.findByTestId('timezone-select');
    expect(screen.getByRole('option', { name: 'UTC' })).toBeTruthy();
  });

  it('a save that failed after core stored it shows the stored zone, without an error', async () => {
    getMock
      .mockResolvedValueOnce(settings('Europe/Berlin'))
      .mockResolvedValueOnce(settings('Asia/Tokyo'));
    updateMock.mockRejectedValue(new Error('response lost'));
    render();
    const select = (await screen.findByTestId('timezone-select')) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe('Europe/Berlin'));
    fireEvent.change(select, { target: { value: 'Asia/Tokyo' } });
    await waitFor(() => expect(select.value).toBe('Asia/Tokyo'));
    expect(screen.queryByTestId('timezone-error')).toBeNull();
  });
});
