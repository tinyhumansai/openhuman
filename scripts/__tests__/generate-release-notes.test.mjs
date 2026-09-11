import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import {
  buildOpenAiRequest,
  buildReleasePayload,
  collectCommits,
  createRecordSplitter,
  ensureAllPullRequestsLinked,
  extractPullRequestNumbers,
  parseArgs,
  parseGitHubRepoFromRemote,
  parseGitLog,
  priorAuthorKeys,
  renderDeterministicNotes,
} from '../release/generate-release-notes.mjs';

test('release notes args default to the latest GitHub release as the start ref', () => {
  assert.equal(parseArgs([]).from, 'latest-release');

  const parsed = parseArgs(['--from', 'v1.0.0', '--to', 'main', '--no-ai']);
  assert.equal(parsed.from, 'v1.0.0');
  assert.equal(parsed.to, 'main');
  assert.equal(parsed.noAi, true);
  assert.equal(parsed.maxPrs, undefined);
});

test('GitHub repo is inferred from ssh and https remotes', () => {
  assert.equal(parseGitHubRepoFromRemote('git@github.com:tinyhumansai/openhuman.git'), 'tinyhumansai/openhuman');
  assert.equal(parseGitHubRepoFromRemote('https://github.com/tinyhumansai/openhuman.git'), 'tinyhumansai/openhuman');
});

test('pull request numbers preserve all linked PRs and pick the last as primary', () => {
  assert.deepEqual(extractPullRequestNumbers('feat(voice): global push-to-talk hotkey (#3090) (#3349)'), [
    3090,
    3349,
  ]);

  const [commit] = parseGitLog(
    'abc123def456\x1ffeat(voice): global push-to-talk hotkey (#3090) (#3349)\x1fCodeGhost21\x1fbot@example.com\x1f2026-06-01T00:00:00Z\x1e',
  );
  assert.equal(commit.primaryPrNumber, 3349);
});

test('OpenAI request contains required release sections and compare payload', () => {
  const payload = buildReleasePayload({
    from: 'v1.0.0',
    to: 'main',
    resolvedTo: 'main',
    repo: 'tinyhumansai/openhuman',
    commits: [],
    contributors: [],
    pullRequests: [],
  });
  const request = buildOpenAiRequest({ model: 'gpt-5.2', title: 'v1.0.0 to main', payload });

  assert.equal(request.model, 'gpt-5.2');
  assert.match(request.input[1].content, /exciting H1 title/);
  assert.match(request.input[1].content, /Do not use the tag range as the title/);
  assert.match(request.input[1].content, /multiple high-level highlight subsections/);
  assert.match(request.input[1].content, /one or two short paragraphs maximum/);
  assert.match(request.input[1].content, /Do not add a "## Pull Requests" section/);
  assert.match(request.input[1].content, /https:\/\/github\.com\/tinyhumansai\/openhuman\/compare\/v1\.0\.0\.\.\.main/);
});

test('OpenAI request compacts release ranges larger than the prompt budget', () => {
  const pullRequests = Array.from({ length: 300 }, (_, index) => ({
    number: index + 1,
    title: `Release improvement ${index + 1}`,
    url: `https://github.com/tinyhumansai/openhuman/pull/${index + 1}`,
    author: `contributor-${index + 1}`,
    labels: ['enhancement'],
    body: 'x'.repeat(700),
    commits: [{ sha: `${index + 1}`, subject: `Release improvement ${index + 1}` }],
  }));
  const payload = buildReleasePayload({
    from: 'v1.0.0',
    to: 'main',
    resolvedTo: 'main',
    repo: 'tinyhumansai/openhuman',
    commits: [],
    contributors: [],
    pullRequests,
  });

  const request = buildOpenAiRequest({ model: 'gpt-5.2', title: 'Large release', payload });
  assert.match(request.input[1].content, /Release improvement 300/);
  assert.doesNotMatch(request.input[1].content, /x{700}/);
});

test('release payload omits contributor emails before AI summarization', () => {
  const payload = buildReleasePayload({
    from: 'v1.0.0',
    to: 'main',
    resolvedTo: 'main',
    repo: 'tinyhumansai/openhuman',
    commits: [],
    contributors: [
      {
        name: 'Privacy First',
        email: 'privacy@example.com',
        commits: 1,
        prs: [12],
        isNew: false,
      },
    ],
    pullRequests: [],
  });

  assert.equal(payload.contributors[0].email, undefined);
  assert.doesNotMatch(JSON.stringify(payload), /privacy@example\.com/);
});

test('deterministic notes credit contributors and link every PR', () => {
  const payload = buildReleasePayload({
    from: 'v1.0.0',
    to: 'main',
    resolvedTo: 'main',
    repo: 'tinyhumansai/openhuman',
    commits: [
      {
        shortSha: 'abc123def',
        subject: 'feat: ship something (#123)',
        prNumbers: [123],
      },
    ],
    contributors: [
      {
        name: 'New Contributor',
        email: 'new@example.com',
        commits: 1,
        prs: [123],
        isNew: true,
      },
    ],
    pullRequests: [
      {
        number: 123,
        title: 'Ship something',
        url: 'https://github.com/tinyhumansai/openhuman/pull/123',
        author: 'newbie',
      },
    ],
  });

  const markdown = renderDeterministicNotes({ title: 'v1.0.0 to main', payload });
  assert.match(markdown, /^# The Intelligence Upgrade/);
  assert.match(markdown, /Welcome New Contributor/);
  assert.match(markdown, /Thank you for \[#123\].*Ship something/);
  assert.match(markdown, /Discord for exclusive roles and contributor rewards/);
  assert.match(markdown, /\[#123\]\(https:\/\/github\.com\/tinyhumansai\/openhuman\/pull\/123\)/);
  const highlightsSection = markdown.split('## New Contributors')[0];
  assert.match(highlightsSection, /Thank you @newbie/);
  assert.match(highlightsSection, /### /);
  assert.doesNotMatch(highlightsSection, /\n- /);
  assert.doesNotMatch(markdown, /## Pull Requests/);
  assert.match(markdown, /🎉/);
});

test('missing model links are appended as an included PR section', () => {
  const markdown = ensureAllPullRequestsLinked('## Highlights\n\nGreat release.\n\n## Contributor Credits\n\nThanks.', [
    {
      number: 42,
      title: 'Fix launch',
      url: 'https://github.com/tinyhumansai/openhuman/pull/42',
      author: 'alice',
    },
  ]);

  assert.match(markdown, /### Additional highlights/);
  assert.match(markdown, /\[#42\]\(https:\/\/github\.com\/tinyhumansai\/openhuman\/pull\/42\).*Thank you @alice/);
  assert.doesNotMatch(markdown.split('## Contributor Credits')[0], /\n- /);
});

test('missing PR detection does not treat prefix matches as exact links', () => {
  const markdown = ensureAllPullRequestsLinked('## Highlights\n\n([#123](https://github.com/tinyhumansai/openhuman/pull/123))', [
    {
      number: 12,
      title: 'Fix prefix collision',
      url: 'https://github.com/tinyhumansai/openhuman/pull/12',
      author: 'alice',
    },
    {
      number: 123,
      title: 'Existing link',
      url: 'https://github.com/tinyhumansai/openhuman/pull/123',
      author: 'bob',
    },
  ]);

  assert.match(markdown, /\[#12\]\(https:\/\/github\.com\/tinyhumansai\/openhuman\/pull\/12\)/);
  assert.equal(markdown.match(/\[#123\]/g)?.length, 1);
});

test('deterministic notes omit new contributors section when there are none', () => {
  const payload = buildReleasePayload({
    from: 'v1.0.0',
    to: 'main',
    resolvedTo: 'main',
    repo: 'tinyhumansai/openhuman',
    commits: [],
    contributors: [
      {
        name: 'Returning Contributor',
        email: 'returning@example.com',
        commits: 1,
        prs: [7],
        isNew: false,
      },
    ],
    pullRequests: [],
  });

  const markdown = renderDeterministicNotes({ title: 'v1.0.0 to main', payload });
  assert.doesNotMatch(markdown, /## New Contributors/);
});

test('record splitter reassembles records across chunk boundaries', () => {
  const records = [];
  const splitter = createRecordSplitter('\x1e', (record) => records.push(record));

  // A separator straddling two chunks is the failure mode a naive
  // `chunk.split(sep)` reader gets wrong, so feed one byte at a time.
  const stream = 'alpha\x1ebeta\x1egamma';
  for (const character of stream) {
    splitter.push(character);
  }
  assert.deepEqual(records, ['alpha', 'beta']);

  // The final record has no trailing separator; end() must still emit it.
  splitter.end();
  assert.deepEqual(records, ['alpha', 'beta', 'gamma']);

  // end() is idempotent — a second call must not re-emit.
  splitter.end();
  assert.deepEqual(records, ['alpha', 'beta', 'gamma']);
});

test('commit collection survives a git log larger than the 1 MiB spawn buffer', async (t) => {
  // Regression guard for the v0.63.21 release failure: `execFileSync` buffers the
  // child's whole stdout and throws `spawnSync git ENOBUFS` past Node's 1 MiB
  // `maxBuffer` default. The release span grows with every unpublished tag, so the
  // collector must not have a fixed output ceiling at all.
  const MAX_BUFFER_BYTES = 1024 * 1024;
  const repo = mkdtempSync(join(tmpdir(), 'release-notes-enobufs-'));
  t.after(() => rmSync(repo, { recursive: true, force: true }));

  // Ignore the ambient git config: a developer's global `commit.gpgsign` or
  // `tag.gpgSign` would otherwise make the fixture prompt or fail.
  const git = (...args) =>
    execFileSync('git', args, {
      cwd: repo,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' },
    });

  git('init', '--quiet', '--initial-branch', 'main');
  git('config', 'user.name', 'Range Fixture');
  git('config', 'user.email', 'range@example.test');
  git('config', 'commit.gpgsign', 'false');
  git('config', 'tag.gpgSign', 'false');
  git('commit', '--allow-empty', '--quiet', '-m', 'base');
  git('tag', 'start');

  const COMMITS = 150;
  const SUBJECT_PADDING = 8000;
  for (let index = 0; index < COMMITS; index += 1) {
    git(
      'commit',
      '--allow-empty',
      '--quiet',
      '-m',
      `chore: padded commit ${index} ${'x'.repeat(SUBJECT_PADDING)} (#${1000 + index})`,
    );
  }
  git('tag', 'end');

  const previousCwd = process.cwd();
  process.chdir(repo);
  t.after(() => process.chdir(previousCwd));

  // The fixture is only a regression guard if it actually overflows the buffer
  // the old implementation used.
  const rawBytes = Buffer.byteLength(
    execFileSync(
      'git',
      ['log', 'start..end', '--reverse', '--format=%H%x1f%s%x1f%an%x1f%ae%x1f%aI%x1e'],
      {
        cwd: repo,
        encoding: 'utf8',
        maxBuffer: 64 * 1024 * 1024,
        env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null' },
      },
    ),
  );
  assert.ok(
    rawBytes > MAX_BUFFER_BYTES,
    `fixture must exceed the 1 MiB spawn buffer, got ${rawBytes} bytes`,
  );

  const commits = await collectCommits('start', 'end');
  assert.equal(commits.length, COMMITS);
  assert.equal(commits.at(0).primaryPrNumber, 1000);
  assert.equal(commits.at(-1).primaryPrNumber, 1000 + COMMITS - 1);
  assert.ok(commits.every((commit) => commit.sha.length === 40));

  const priorKeys = await priorAuthorKeys('start');
  assert.ok(priorKeys.has('range fixture <range@example.test>'));
});
