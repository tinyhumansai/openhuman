import assert from 'node:assert/strict';
import test from 'node:test';

import { lint, render } from '../ci/check-storage-schema.mjs';

const collection = (name, extra = {}) => ({
  name,
  indexes: [],
  ...extra,
});

test('a well-formed per-crate schema is clean', () => {
  const dump = {
    owners: [{
      crate: 'tinyflows-scheduler',
      collections: [collection('cron.jobs', {
        indexes: [{ name: 'by_time', fields: ['next_run_ms'], unique: false }],
        ttl_field: 'expires_ms',
        search: { fields: ['name', 'description'] },
      })],
    }],
  };
  assert.deepEqual(lint(dump), []);
});

test('a flattened schema dump is rejected instead of passing as empty', () => {
  assert.deepEqual(lint({ collections: [collection('cron.jobs')] }), [
    'schema dump must group collections under an owners array',
  ]);
});

test('collection names must be <owner>.<entity> in snake_case', () => {
  const problems = lint({
    owners: [{ crate: 'openhuman', collections: [
      collection('jobs'),
      collection('Cron.Jobs'),
      collection('flows.runs-v2'),
    ] }],
  });
  assert.equal(problems.length, 3, problems.join('\n'));
});

test('legacy migration source names are allowed only for TinyChannels storage reads', () => {
  assert.deepEqual(lint({
    owners: [{ crate: 'tinychannels-bus', collections: [collection('channels_delivery_pending')] }],
  }), []);
  assert.equal(lint({
    owners: [{ crate: 'tinychannels-bus', collections: [collection('channels_new_collection')] }],
  }).length, 1);
  assert.equal(lint({
    owners: [{ crate: 'openhuman', collections: [collection('channels_delivery_pending')] }],
  }).length, 1);
});

test('duplicate collection and index names are refused within one crate', () => {
  const problems = lint({
    owners: [{ crate: 'tinyflows-scheduler', collections: [
      collection('cron.jobs', {
        indexes: [
          { name: 'by_time', fields: ['created_ms'] },
          { name: 'by_time', fields: ['updated_ms'] },
        ],
      }),
      collection('cron.jobs'),
    ] }],
  });
  assert.ok(problems.some(p => p.includes('collection "cron.jobs" is declared more than once')));
  assert.ok(problems.some(p => p.includes('index name "by_time" is empty or duplicated')));
});

test('indexed time fields must be epoch milliseconds', () => {
  const problems = lint({
    owners: [{ crate: 'tinyflows-scheduler', collections: [collection('cron.jobs', {
      indexes: [{ name: 'by_time', fields: ['created_at', 'next_run_ms'] }],
      ttl_field: 'expires_at',
    })] }],
  });
  assert.equal(problems.length, 2);
  assert.match(problems[0], /created_at/);
  assert.match(problems[1], /expires_at/);
});

test('the rendering keeps CollectionSpecs grouped by their declaring crate', () => {
  const dump = {
    owners: [
      { crate: 'tinyagents-session', collections: [collection('a.y', {
        indexes: [{ name: 'by_created', fields: ['created_ms'], unique: true }],
        search: { fields: ['title'] },
      }), collection('a.z', { ttl_field: 'expires_ms' })] },
      { crate: 'openhuman', collections: [collection('a.y'), collection('remix_markers')] },
      { crate: 'tinyflows', collections: [collection('b.x')] },
    ],
  };
  const text = render(dump);
  assert.ok(text.indexOf('## openhuman') < text.indexOf('## tinyagents-session'));
  assert.ok(text.indexOf('## tinyagents-session') < text.indexOf('## tinyflows'));
  assert.match(text, /## tinyagents-session\n/);
  assert.match(text, /## tinyflows\n/);
  assert.match(text, /## openhuman\n/);
  assert.equal((text.match(/\| `a\.y` \|/g) ?? []).length, 2);
  assert.doesNotMatch(text, /composed registry/i);
  assert.match(text, /by_created/);
  assert.match(text, /`expires_ms`/);
  assert.match(text, /`title`/);
  assert.equal(render(dump), text);
});

test('the dump example builds with web3 by default and with an override that keeps it', async () => {
  const { dumpFeatures, dumpSchema } = await import('../ci/check-storage-schema.mjs');
  assert.equal(dumpFeatures({}), 'web3');
  assert.equal(dumpFeatures({ STORAGE_SCHEMA_FEATURES: 'web3,flows' }), 'web3,flows');
  assert.throws(() => dumpFeatures({ STORAGE_SCHEMA_FEATURES: 'flows' }), /web3/);
  const product = dumpFeatures({ STORAGE_SCHEMA_FEATURES: 'core-product' });
  assert.ok(product.split(',').includes('web3'));
  let args;
  const dump = dumpSchema(undefined, {
    env: { STORAGE_SCHEMA_FEATURES: 'core-product' },
    run: (_cmd, argv) => {
      args = argv;
      return { status: 0, stdout: '{"owners":[]}', stderr: '' };
    },
  });
  assert.deepEqual(dump, { owners: [] });
  assert.equal(args[args.indexOf('--features') + 1], product);
});
