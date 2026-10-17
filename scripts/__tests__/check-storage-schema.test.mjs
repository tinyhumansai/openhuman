import assert from 'node:assert/strict';
import test from 'node:test';

import { lint, render } from '../ci/check-storage-schema.mjs';

const owner = (name, items, steps = []) => ({ owner: name, items, steps });
const collection = (name, extra = {}) => ({
  kind: 'collection',
  name,
  version: 2,
  scope: 'tenant',
  index_fields: [],
  ...extra,
});

test('a well-formed schema is clean', () => {
  const dump = {
    owners: [
      owner('cron', [collection('cron.jobs', { index_fields: ['next_run_ms'] })], [
        { id: 'jobs_v2', item: 'cron.jobs', from: 1, to: 2, source: 'cron_jobs' },
      ]),
      owner('juice', [{ kind: 'blobs', name: 'juice/ccr/', scope: 'module' }]),
    ],
  };
  assert.deepEqual(lint(dump), []);
});

test('names must be <owner>.<entity> in snake_case without a version suffix', () => {
  const problems = lint({
    owners: [
      owner('cron', [
        collection('jobs'),
        collection('cron.Jobs'),
        collection('flows.runs'),
        collection('cron.jobs_v2'),
      ]),
    ],
  });
  assert.equal(problems.length, 4, problems.join('\n'));
});

test('a name claimed by two owners is refused', () => {
  const problems = lint({
    owners: [owner('a', [{ kind: 'blobs', name: 'a/x/', scope: 'module' }]), owner('a', [])],
  });
  assert.ok(problems.some(p => p.includes('declared twice')));
});

test('indexed time fields must be epoch milliseconds', () => {
  const problems = lint({
    owners: [owner('cron', [collection('cron.jobs', { index_fields: ['created_at', 'next_run_ms'] })])],
  });
  assert.equal(problems.length, 1);
  assert.match(problems[0], /created_at/);
});

test('a step for an undeclared item or one that does not go up is refused', () => {
  const problems = lint({
    owners: [
      owner('cron', [collection('cron.jobs')], [
        { id: 'a', item: 'cron.nope', from: 1, to: 2 },
        { id: 'b', item: 'cron.jobs', from: 2, to: 2 },
      ]),
    ],
  });
  assert.equal(problems.length, 2);
});

test('the rendering is stable and lists every owner', () => {
  const dump = { owners: [owner('b', [collection('b.x')]), owner('a', [collection('a.y')])] };
  const text = render(dump);
  assert.ok(text.indexOf('## a') < text.indexOf('## b'));
  assert.equal(render(dump), text);
});
