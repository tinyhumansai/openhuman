import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * `CronJobsPanel` measured 97.5% lines but only 60.9% branches. The existing
 * `CronJobsPanel.test.tsx` drives every handler's happy path and every failure
 * path where the rejection is an `Error`. Two things it never reaches:
 *
 *   - the `next_run` sort comparator in `loadCoreCronJobs` (panel :49-52),
 *     an uncovered function. It decides which job the user reads as "next",
 *     and the list arrives from the core in arbitrary order.
 *   - the `: String(err)` arm of the eight
 *     `err instanceof Error ? err.message : String(err)` expressions
 *     (:67, :92, :110, :147, :158, :169, :190). A core RPC that rejects with a
 *     bare string — which the JSON-RPC layer does — must still produce readable
 *     copy rather than `[object Object]` or a blank error row.
 *
 * Beside the existing file rather than inside it so the two stay independently
 * reviewable; the mock surface is deliberately identical.
 */

// `formatCronError` is `t(key).replace('{message}', message)`, so a `t` that
// returns the bare key silently DROPS the message — which is why the existing
// suite asserts on keys alone. Returning a real template is what makes the
// `String(err)` arm observable at all.
const stableI18n = { t: (k: string) => `${k}: {message}` };
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => stableI18n }));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

const noopDispatch = vi.fn();
vi.mock('../../../store/hooks', () => ({
  useAppDispatch: () => noopDispatch,
  useAppSelector: () => [],
}));

vi.mock('./cron/CronJobFormModal', () => ({
  default: ({ open }: { open: boolean }) => (open ? <div data-testid="cron-modal" /> : null),
}));

const cronAddMock = vi.fn();
const cronListMock = vi.fn();
const cronUpdateMock = vi.fn();
const cronRemoveMock = vi.fn();
const cronRunMock = vi.fn();
const cronRunsMock = vi.fn();

vi.mock('../../../utils/tauriCommands', () => ({
  openhumanCronAdd: (...args: unknown[]) => cronAddMock(...args),
  openhumanCronList: () => cronListMock(),
  openhumanCronUpdate: (...args: unknown[]) => cronUpdateMock(...args),
  openhumanCronRemove: (...args: unknown[]) => cronRemoveMock(...args),
  openhumanCronRun: (...args: unknown[]) => cronRunMock(...args),
  openhumanCronRuns: (...args: unknown[]) => cronRunsMock(...args),
}));

function job(over: Record<string, unknown> = {}) {
  return {
    id: 'job-1',
    expression: '*/30 * * * *',
    schedule: { kind: 'cron', expr: '*/30 * * * *' },
    command: '',
    name: 'Daily Briefing',
    job_type: 'agent',
    session_target: 'isolated',
    enabled: true,
    delivery: { mode: 'proactive', best_effort: true },
    delete_after_run: false,
    created_at: '2026-05-01T00:00:00.000Z',
    next_run: '2026-06-01T09:00:00.000Z',
    prompt: 'Summarise the news',
    ...over,
  };
}

async function importPanel() {
  vi.resetModules();
  const mod = await import('./CronJobsPanel');
  return mod.default;
}

beforeEach(() => {
  [cronListMock, cronAddMock, cronUpdateMock, cronRemoveMock, cronRunMock, cronRunsMock].forEach(
    fn => fn.mockReset()
  );
  cronListMock.mockResolvedValue({ result: [job()] });
  cronAddMock.mockResolvedValue({ result: job({ id: 'job-new' }) });
  cronUpdateMock.mockResolvedValue({ result: job() });
  cronRemoveMock.mockResolvedValue({ result: { job_id: 'job-1', removed: true } });
  cronRunMock.mockResolvedValue({ result: {} });
  cronRunsMock.mockResolvedValue({ result: [] });
});

describe('CronJobsPanel — next_run ordering', () => {
  /** Read the rendered job names in DOM order. */
  function renderedNames(names: string[]): string[] {
    return names.filter(n => screen.queryByText(n) !== null);
  }

  it('renders jobs soonest-first regardless of the order the core returned', async () => {
    cronListMock.mockResolvedValue({
      result: [
        job({ id: 'c', name: 'Latest', next_run: '2026-06-03T09:00:00.000Z' }),
        job({ id: 'a', name: 'Soonest', next_run: '2026-06-01T09:00:00.000Z' }),
        job({ id: 'b', name: 'Middle', next_run: '2026-06-02T09:00:00.000Z' }),
      ],
    });
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(screen.getByText('Soonest')).toBeInTheDocument());

    const body = document.body.textContent ?? '';
    expect(body.indexOf('Soonest')).toBeLessThan(body.indexOf('Middle'));
    expect(body.indexOf('Middle')).toBeLessThan(body.indexOf('Latest'));
    expect(renderedNames(['Soonest', 'Middle', 'Latest'])).toHaveLength(3);
  });

  it('keeps the already-sorted case sorted', async () => {
    cronListMock.mockResolvedValue({
      result: [
        job({ id: 'a', name: 'First', next_run: '2026-06-01T00:00:00.000Z' }),
        job({ id: 'b', name: 'Second', next_run: '2026-06-02T00:00:00.000Z' }),
      ],
    });
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(screen.getByText('First')).toBeInTheDocument());

    const body = document.body.textContent ?? '';
    expect(body.indexOf('First')).toBeLessThan(body.indexOf('Second'));
  });

  it('does not drop a job whose next_run is unparseable', async () => {
    // `new Date('not-a-date').getTime()` is NaN, so the comparator returns NaN
    // for that pair. The row must still render — a schedule vanishing from the
    // list because the core sent an odd timestamp would be silent data loss.
    cronListMock.mockResolvedValue({
      result: [
        job({ id: 'a', name: 'Valid', next_run: '2026-06-01T00:00:00.000Z' }),
        job({ id: 'b', name: 'Unparseable', next_run: 'not-a-date' }),
      ],
    });
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(screen.getByText('Valid')).toBeInTheDocument());
    expect(screen.getByText('Unparseable')).toBeInTheDocument();
  });
});

describe('CronJobsPanel — rejections that are not Error instances', () => {
  /**
   * The core's JSON-RPC client rejects with a plain string on some paths. Each
   * handler formats it through `err instanceof Error ? err.message : String(err)`;
   * the `String(err)` arm is what these cover.
   */
  it('renders a string rejection from the list call', async () => {
    cronListMock.mockRejectedValue('core unreachable');
    const Panel = await importPanel();
    render(<Panel />);
    expect(await screen.findByText(/core unreachable/)).toBeInTheDocument();
  });

  it('stringifies a non-Error object rejection rather than showing an empty error', async () => {
    cronListMock.mockRejectedValue({ code: -32000 });
    const Panel = await importPanel();
    render(<Panel />);
    // `String({})` is "[object Object]"; this pins that the panel stringifies
    // the rejection rather than swallowing it into an empty error row.
    expect(await screen.findByText(/\[object Object\]/)).toBeInTheDocument();
  });

  it('renders a string rejection from the toggle call', async () => {
    cronUpdateMock.mockRejectedValue('toggle refused');
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(screen.getByText('Daily Briefing')).toBeInTheDocument());

    fireEvent.click(await screen.findByTestId('cron-job-toggle-job-1'));
    expect(await screen.findByText(/toggle refused/)).toBeInTheDocument();
  });
});
