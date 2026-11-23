import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { CoreCronJob } from '../../../../utils/tauriCommands/cron';
import CronJobFormModal from './CronJobFormModal';

/**
 * Edit mode must change only what the user changed. Jobs created from a chat
 * turn carry routing the form cannot represent (`delivery.mode: origin` /
 * `announce` with `channel`/`to`, `session_target: current`, a cron `tz` and
 * `active_hours`). The core replaces `schedule` and `delivery` wholesale on
 * update, so a patch that rebuilds them from the form's narrower state silently
 * detaches the job from its conversation and moves it to the machine's zone.
 */

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function job(over: Partial<CoreCronJob> = {}): CoreCronJob {
  return {
    id: 'job-1',
    expression: '0 9 * * 1-5',
    schedule: {
      kind: 'cron',
      expr: '0 9 * * 1-5',
      tz: 'Asia/Tokyo',
      active_hours: { start: '08:00', end: '18:00' },
    },
    command: '',
    name: 'Standup reminder',
    job_type: 'agent',
    session_target: 'current',
    enabled: true,
    delivery: { mode: 'origin', channel: null, to: null, best_effort: true },
    delete_after_run: false,
    created_at: '2026-11-01T00:00:00.000Z',
    next_run: '2026-11-24T00:00:00.000Z',
    prompt: 'Remind me about standup',
    ...over,
  };
}

function renderEdit(stored: CoreCronJob) {
  const onUpdate = vi.fn().mockResolvedValue(undefined);
  render(
    <CronJobFormModal
      mode="edit"
      job={stored}
      open
      onClose={vi.fn()}
      onCreate={vi.fn()}
      onUpdate={onUpdate}
    />
  );
  return onUpdate;
}

/**
 * Picks an option the way a browser does. A native `<select>` fires no `change`
 * event when the user confirms the option it already shows, so a test that
 * fires `change` with the displayed value would pass on a form the user cannot
 * actually drive.
 */
function pick(testId: string, value: string) {
  const select = screen.getByTestId(testId) as HTMLSelectElement;
  if (select.value !== value) fireEvent.change(select, { target: { value } });
}

function optionValues(testId: string) {
  return Array.from((screen.getByTestId(testId) as HTMLSelectElement).options).map(o => o.value);
}

async function submittedPatch(onUpdate: ReturnType<typeof vi.fn>) {
  fireEvent.click(screen.getByTestId('cron-form-submit'));
  await waitFor(() => expect(onUpdate).toHaveBeenCalledTimes(1));
  return onUpdate.mock.calls[0][1] as Record<string, unknown>;
}

describe('CronJobFormModal edit mode', () => {
  it('keeps origin delivery, current session target, tz and active hours on a rename', async () => {
    const onUpdate = renderEdit(job());
    fireEvent.change(screen.getByTestId('cron-form-name'), { target: { value: 'Standup' } });

    const patch = await submittedPatch(onUpdate);

    expect(onUpdate.mock.calls[0][0]).toBe('job-1');
    expect(patch.name).toBe('Standup');
    expect(patch).not.toHaveProperty('delivery');
    expect(patch).not.toHaveProperty('session_target');
    expect(patch.schedule).toEqual({
      kind: 'cron',
      expr: '0 9 * * 1-5',
      tz: 'Asia/Tokyo',
      active_hours: { start: '08:00', end: '18:00' },
    });
  });

  it('keeps an announce route and the stored zone when only the cron expression changes', async () => {
    const onUpdate = renderEdit(
      job({
        session_target: 'isolated',
        delivery: { mode: 'announce', channel: 'telegram', to: '12345', best_effort: true },
      })
    );
    fireEvent.change(screen.getByTestId('cron-form-cron-custom'), {
      target: { value: '30 9 * * 1-5' },
    });

    const patch = await submittedPatch(onUpdate);

    expect(patch).not.toHaveProperty('delivery');
    expect(patch).not.toHaveProperty('session_target');
    expect(patch.schedule).toEqual({
      kind: 'cron',
      expr: '30 9 * * 1-5',
      tz: 'Asia/Tokyo',
      active_hours: { start: '08:00', end: '18:00' },
    });
  });

  it('shows the stored routing as its own option instead of a fallback', () => {
    renderEdit(job());

    const delivery = screen.getByTestId('cron-form-delivery') as HTMLSelectElement;
    const session = screen.getByTestId('cron-form-session-target') as HTMLSelectElement;
    expect(delivery.value).not.toBe('none');
    expect(session.value).not.toBe('isolated');
    expect(optionValues('cron-form-delivery')).toEqual([delivery.value, 'proactive', 'none']);
    expect(optionValues('cron-form-session-target')).toEqual([session.value, 'isolated', 'main']);
    expect(delivery.selectedOptions[0].textContent).toBe('settings.cron.jobs.formKeepStored');
  });

  it('offers no keep option when the stored routing is one the form shows', () => {
    renderEdit(job({ session_target: 'main', delivery: { mode: 'proactive', best_effort: true } }));

    expect(optionValues('cron-form-delivery')).toEqual(['proactive', 'none']);
    expect(optionValues('cron-form-session-target')).toEqual(['isolated', 'main']);
  });

  it('sends delivery and session target when the user changes them', async () => {
    const onUpdate = renderEdit(job());
    pick('cron-form-delivery', 'proactive');
    pick('cron-form-session-target', 'main');

    const patch = await submittedPatch(onUpdate);

    expect(patch.delivery).toEqual({ mode: 'proactive', best_effort: true });
    expect(patch.session_target).toBe('main');
  });

  it('sends none and isolated when the user picks them directly to detach the job', async () => {
    const onUpdate = renderEdit(job());
    pick('cron-form-delivery', 'none');
    pick('cron-form-session-target', 'isolated');

    const patch = await submittedPatch(onUpdate);

    expect(patch.delivery).toEqual({ mode: 'none', best_effort: true });
    expect(patch.session_target).toBe('isolated');
  });

  it('keeps the stored routing when the user goes back to the keep option', async () => {
    const onUpdate = renderEdit(job());
    const keepDelivery = (screen.getByTestId('cron-form-delivery') as HTMLSelectElement).value;
    const keepSession = (screen.getByTestId('cron-form-session-target') as HTMLSelectElement).value;
    pick('cron-form-delivery', 'proactive');
    pick('cron-form-delivery', keepDelivery);
    pick('cron-form-session-target', 'main');
    pick('cron-form-session-target', keepSession);

    const patch = await submittedPatch(onUpdate);

    expect(patch).not.toHaveProperty('delivery');
    expect(patch).not.toHaveProperty('session_target');
  });

  it('does not re-send routing the form shows when it is left unchanged', async () => {
    const onUpdate = renderEdit(
      job({ session_target: 'main', delivery: { mode: 'proactive', best_effort: false } })
    );
    pick('cron-form-delivery', 'none');
    pick('cron-form-delivery', 'proactive');

    const patch = await submittedPatch(onUpdate);

    expect(patch).not.toHaveProperty('delivery');
    expect(patch).not.toHaveProperty('session_target');
  });

  it('drops tz and active hours when the schedule switches away from cron', async () => {
    const onUpdate = renderEdit(job());
    fireEvent.click(screen.getByTestId('cron-form-schedule-every'));
    fireEvent.change(screen.getByTestId('cron-form-every'), { target: { value: '3600000' } });

    const patch = await submittedPatch(onUpdate);

    expect(patch.schedule).toEqual({ kind: 'every', every_ms: 3600000 });
  });

  it('still builds a zone-less cron schedule with explicit delivery in create mode', async () => {
    const onCreate = vi.fn().mockResolvedValue(undefined);
    render(
      <CronJobFormModal
        mode="create"
        open
        onClose={vi.fn()}
        onCreate={onCreate}
        onUpdate={vi.fn()}
      />
    );
    fireEvent.change(screen.getByTestId('cron-form-prompt'), { target: { value: 'Summarise' } });
    fireEvent.click(screen.getByTestId('cron-form-submit'));

    await waitFor(() => expect(onCreate).toHaveBeenCalledTimes(1));
    const params = onCreate.mock.calls[0][0] as Record<string, unknown>;
    expect(params.schedule).toMatchObject({ kind: 'cron', tz: null });
    expect(params.schedule).not.toHaveProperty('active_hours');
    expect(params.delivery).toEqual({ mode: 'proactive', best_effort: true });
    expect(params.session_target).toBe('isolated');
  });
});
