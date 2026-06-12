import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  buildOpenAiRequest,
  buildReleasePayload,
  ensureAllPullRequestsLinked,
  extractPullRequestNumbers,
  parseArgs,
  parseGitHubRepoFromRemote,
  parseGitLog,
  renderDeterministicNotes,
} from '../release/generate-release-notes.mjs';

test('release notes args default to the latest GitHub release as the start ref', () => {
  assert.equal(parseArgs([]).from, 'latest-release');

  const parsed = parseArgs(['--from', 'v1.0.0', '--to', 'main', '--no-ai', '--max-prs', '5']);
  assert.equal(parsed.from, 'v1.0.0');
  assert.equal(parsed.to, 'main');
  assert.equal(parsed.noAi, true);
  assert.equal(parsed.maxPrs, 5);
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
