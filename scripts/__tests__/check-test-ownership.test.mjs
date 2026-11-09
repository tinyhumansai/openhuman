import assert from 'node:assert/strict';
import test from 'node:test';

import { findOwnershipViolations } from '../ci/check-test-ownership.mjs';

test('rejects tests that call TinyChannels provider internals', () => {
  const source = `
    use tinychannels::providers::email_channel::parse;
    #[test]
    fn parses_email() { let _ = parse("message"); }
  `;

  assert.deepEqual(findOwnershipViolations(source, 'tests/email.rs'), [
    {
      line: 2,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
  ]);
});

test('rejects grouped imports of TinyChannels provider internals', () => {
  const source = `
    use tinychannels::{providers::email_channel::parse};
  `;

  assert.deepEqual(findOwnershipViolations(source, 'tests/email.rs'), [
    {
      line: 2,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
  ]);
});

test('rejects TinyChannels provider imports with whitespace around path separators', () => {
  const source = `
    use tinychannels :: providers :: email_channel :: parse;
  `;

  assert.deepEqual(findOwnershipViolations(source, 'tests/email.rs'), [
    {
      line: 2,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
  ]);
});

test('rejects direct calls into TinyChannels provider internals', () => {
  const source = `
    tinychannels::providers::email_channel::parse("message");
  `;

  assert.deepEqual(findOwnershipViolations(source, 'tests/email.rs'), [
    {
      line: 2,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
  ]);
});

test('rejects direct calls through a crate alias', () => {
  const source = [
    'use tinychannels as tc;',
    'tc::providers::email_channel::parse("message");',
  ].join('\n');

  assert.deepEqual(findOwnershipViolations(source, 'tests/email.rs'), [
    {
      line: 2,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
  ]);
});

test('rejects direct calls through an alias of a grouped implementation prefix', () => {
  const source = [
    'use tinychannels::{providers as p};',
    'p::email_channel::parse("message");',
  ].join('\n');

  assert.deepEqual(findOwnershipViolations(source, 'tests/email.rs'), [
    {
      line: 1,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
    {
      line: 2,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
  ]);
});

test('resolves chained crate and implementation-prefix aliases', () => {
  const source = [
    'use tinychannels as tc;',
    'use tc::providers as p;',
    'p::email_channel::parse("message");',
  ].join('\n');

  assert.deepEqual(findOwnershipViolations(source, 'tests/email.rs'), [
    {
      line: 2,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
    {
      line: 3,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
  ]);
});

test('expands nested grouped imports of TinyChannels provider internals', () => {
  const source = `
    use tinychannels::{providers::{email_channel::parse}};
  `;

  assert.deepEqual(findOwnershipViolations(source, 'tests/email.rs'), [
    {
      line: 2,
      path: 'tinychannels::providers::',
      reason: 'TinyChannels provider behavior belongs in TinyChannels.',
    },
  ]);
});

test('allowlisted implementation paths do not permit direct calls outside their fixture', () => {
  const source = 'tinymemory_tools::lifecycle::window::Window::default();';
  assert.equal(
    findOwnershipViolations(source, 'crates/openhuman-core/src/example_tests.rs').length,
    1,
  );
});

test('allows public facades and bus contracts', () => {
  const source = `
    use tinychannels::ChannelInboundEnvelope;
    use tinychannels_bus::contract::SendMessageRequest;
    use tinyinference_local::profile::is_local_provider_string;
  `;

  assert.deepEqual(findOwnershipViolations(source, 'crates/openhuman-core/src/example_tests.rs'), []);
});

test('limits reasoned implementation exceptions to their named host test', () => {
  const source = [
    'use tinymemory_tools::lifecycle::turn::TurnPack;',
    'use tinymemory_tools::lifecycle::window::Window;',
  ].join('\n');

  assert.deepEqual(
    findOwnershipViolations(source, 'crates/openhuman-core/src/memory/lifecycle/adapter_tests.rs'),
    [],
  );
  assert.deepEqual(
    findOwnershipViolations(source, 'crates/openhuman-core/src/example_tests.rs').map(
      ({ path }) => path,
    ),
    ['tinymemory_tools::lifecycle::turn::', 'tinymemory_tools::lifecycle::window::'],
  );
});

test('rejects other known module implementation paths', () => {
  const source = [
    'use tinyinference_local::service::LocalAiService;',
    'use tinyflows::gates::failures;',
    'use tinymemory_tools::lifecycle::turn::TurnPack;',
  ].join('\n');

  assert.deepEqual(
    findOwnershipViolations(source, 'crates/openhuman-core/src/example_tests.rs').map(
      ({ path }) => path,
    ),
    [
      'tinyinference_local::service::',
      'tinyflows::gates::',
      'tinymemory_tools::lifecycle::turn::',
    ],
  );
});

test('does not scan production Rust files', () => {
  const source = 'use tinychannels::providers::email_channel::parse;';
  assert.deepEqual(findOwnershipViolations(source, 'crates/openhuman-core/src/example.rs'), []);
});

test('ignores ownership examples in Rust comments', () => {
  const source = '// tinychannels::providers::email_channel::parse is forbidden\n';
  assert.deepEqual(findOwnershipViolations(source, 'tests/example.rs'), []);
});
