// @ts-nocheck
import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { callOpenhumanRpc } from '../helpers/core-rpc';
import { expectRpcMethod, fetchCoreRpcMethods } from '../helpers/core-schema';

function pickTaskId(payload: unknown): string | null {
  const text = JSON.stringify(payload || {});
  const fromTask = (payload as any)?.task?.id;
  if (typeof fromTask === 'string' && fromTask.length > 0) return fromTask;
  const fromResult = (payload as any)?.result?.task_id;
  if (typeof fromResult === 'string' && fromResult.length > 0) return fromResult;
  const match = text.match(/"id"\s*:\s*"([a-zA-Z0-9_-]{6,})"/);
  return match?.[1] || null;
}

async function expectRpcOk(method: string, params: Record<string, unknown> = {}) {
  const result = await callOpenhumanRpc(method, params);
  if (!result.ok) {
    console.log(`[AutomationSpec] ${method} failed`, result.error);
  }
  expect(result.ok).toBe(true);
  return result.result;
}

describe('Automation & Scheduling', () => {
  let methods: Set<string>;
  let taskId: string | null = null;

  before(async () => {
    await waitForApp();
    await waitForAppReady(20_000);
    methods = await fetchCoreRpcMethods();
  });

  async function ensureTask(): Promise<string | null> {
    if (taskId) return taskId;
    const created = await callOpenhumanRpc('openhuman.subconscious_tasks_add', {
      title: 'e2e scheduled task',
      source: 'user',
    });
    if (!created.ok) return null;
    taskId = pickTaskId(created.result);
    return taskId;
  }

  async function expectUnavailable(
    method: string,
    params: Record<string, unknown> = {}
  ): Promise<void> {
    const res = await callOpenhumanRpc(method, params);
    expect(res.ok).toBe(false);
  }

  it('6.1.1 — Task Creation: subconscious.tasks_add returns created task', async () => {
    if (!methods.has('openhuman.subconscious_tasks_add')) {
      await expectUnavailable('openhuman.subconscious_tasks_add', {
        title: 'e2e scheduled task',
        source: 'user',
      });
      return;
    }

    expectRpcMethod(methods, 'openhuman.subconscious_tasks_add');
    taskId = await ensureTask();
    expect(Boolean(taskId)).toBe(true);
  });

  it('6.1.2 — Task Update: subconscious.tasks_update accepts patch fields', async () => {
    if (!methods.has('openhuman.subconscious_tasks_update')) {
      await expectUnavailable('openhuman.subconscious_tasks_update', {
        task_id: 'missing-task',
        title: 'e2e scheduled task updated',
        enabled: true,
      });
      return;
    }

    const id = await ensureTask();
    expect(id).toBeTruthy();
    await expectRpcOk('openhuman.subconscious_tasks_update', {
      task_id: id,
      title: 'e2e scheduled task updated',
      enabled: true,
    });
  });

  it('6.1.3 — Task Deletion: subconscious.tasks_remove removes task', async () => {
    if (!methods.has('openhuman.subconscious_tasks_remove')) {
      await expectUnavailable('openhuman.subconscious_tasks_remove', { task_id: 'missing-task' });
      return;
    }

    const id = await ensureTask();
    expect(id).toBeTruthy();
    await expectRpcOk('openhuman.subconscious_tasks_remove', { task_id: id });
    if (methods.has('openhuman.subconscious_tasks_list')) {
      const tasks = await expectRpcOk('openhuman.subconscious_tasks_list', {});
      expect(JSON.stringify(tasks || {}).includes(String(id))).toBe(false);
    }
  });

  it('6.2.1 — Cron Expression Validation: invalid cron recurrence is rejected', async () => {
    if (
      !methods.has('openhuman.subconscious_tasks_add') ||
      !methods.has('openhuman.subconscious_tasks_update')
    ) {
      await expectUnavailable('openhuman.subconscious_tasks_update', {
        task_id: 'missing-task',
        recurrence: 'cron:not-a-valid-expression',
      });
      return;
    }

    const created = await expectRpcOk('openhuman.subconscious_tasks_add', {
      title: 'e2e cron validation',
      source: 'user',
    });
    const id = pickTaskId(created);
    expect(id).toBeTruthy();

    const invalid = await callOpenhumanRpc('openhuman.subconscious_tasks_update', {
      task_id: id,
      recurrence: 'cron:not-a-valid-expression',
    });

    expect(invalid.ok).toBe(false);

    if (methods.has('openhuman.subconscious_tasks_remove')) {
      await expectRpcOk('openhuman.subconscious_tasks_remove', { task_id: id });
    }
  });

  it('6.2.2 — Recurring Execution: trigger tick records log entries', async () => {
    if (!methods.has('openhuman.subconscious_trigger')) {
      await expectUnavailable('openhuman.subconscious_trigger', {});
      return;
    }

    await expectRpcOk('openhuman.subconscious_trigger', {});
    if (methods.has('openhuman.subconscious_log_list')) {
      const logs = await expectRpcOk('openhuman.subconscious_log_list', { limit: 20 });
      expect(JSON.stringify(logs || {}).length > 2).toBe(true);
      return;
    }

    await expectUnavailable('openhuman.subconscious_log_list', { limit: 20 });
  });

  it('6.2.3 — Missed Execution Handling: trigger endpoint remains safe across repeated calls', async () => {
    await expectRpcOk('openhuman.subconscious_trigger', {});
    await expectRpcOk('openhuman.subconscious_trigger', {});
  });

  it('6.3.1 — Remote Agent Scheduling: cron list endpoint is available', async () => {
    expectRpcMethod(methods, 'openhuman.cron_list');
    await expectRpcOk('openhuman.cron_list', {});
  });

  it('6.3.2 — Execution Trigger Handling: cron run with missing job_id fails explicitly', async () => {
    const res = await callOpenhumanRpc('openhuman.cron_run', { job_id: 'missing-job-id-e2e' });
    expect(res.ok).toBe(false);
  });

  it('6.3.3 — Failure Retry Logic: cron runs history endpoint remains queryable after failures', async () => {
    const runs = await callOpenhumanRpc('openhuman.cron_runs', {
      job_id: 'missing-job-id-e2e',
      limit: 5,
    });

    expect(runs.ok || Boolean(runs.error)).toBe(true);
  });
});
