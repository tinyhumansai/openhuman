#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import { existsSync, writeFileSync } from 'node:fs';
import { basename, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const DEFAULT_MODEL = 'gpt-5.2';
const DEFAULT_FROM = 'latest-release';
const DEFAULT_TO = 'latest-tag';
const MAX_PROMPT_CHARS = 120_000;
const MAX_PR_BODY_CHARS = 700;

function usage() {
  return `Usage: node scripts/release/generate-release-notes.mjs [options]

Generate Markdown release notes from merged PRs between two refs.

Options:
  --from <tag>          Start tag/ref, excluded from the range. Defaults to latest-release.
                         Use latest-release to resolve the most recent GitHub Release tag.
  --to <ref>            End ref, included. Defaults to latest-tag. Use main for testing.
  --repo <owner/repo>   GitHub repo. Defaults to upstream/origin remote.
  --model <model>       OpenAI model. Defaults to ${DEFAULT_MODEL}.
  --output <file>       Write generated Markdown to a file instead of stdout.
  --no-ai               Build deterministic Markdown without calling OpenAI.
  --dry-run             Print the OpenAI prompt/input JSON without calling OpenAI.
  --max-prs <n>         Refuse ranges with more than n PRs. Defaults to 250.
  --help                Show this help.

Environment:
  OPENAI_API_KEY        Preferred OpenAI API key variable.
  OPENAI_API            Backward-compatible fallback API key variable.
  OPENAI_MODEL          Overrides the default model.

Examples:
  pnpm release:notes -- --to main --output a.md
  pnpm release:notes -- --from latest-release --to main --no-ai
  pnpm release:notes -- --from v0.57.18 --to main --no-ai
`;
}

export function parseArgs(argv) {
  if (argv[0] === '--') {
    argv = argv.slice(1);
  }

  const options = {
    from: process.env.RELEASE_NOTES_FROM || DEFAULT_FROM,
    to: process.env.RELEASE_NOTES_TO || DEFAULT_TO,
    repo: process.env.GITHUB_REPOSITORY || null,
    model: process.env.OPENAI_MODEL || DEFAULT_MODEL,
    output: null,
    noAi: false,
    dryRun: false,
    maxPrs: 250,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const readValue = (name) => {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        throw new Error(`${name} requires a value`);
      }
      index += 1;
      return value;
    };

    if (arg === '--help' || arg === '-h') {
      return { ...options, help: true };
    }
    if (arg === '--from') {
      options.from = readValue(arg);
    } else if (arg === '--to') {
      options.to = readValue(arg);
    } else if (arg === '--repo') {
      options.repo = readValue(arg);
    } else if (arg === '--model') {
      options.model = readValue(arg);
    } else if (arg === '--output' || arg === '-o') {
      options.output = readValue(arg);
    } else if (arg === '--max-prs') {
      const parsed = Number.parseInt(readValue(arg), 10);
      if (!Number.isInteger(parsed) || parsed < 1) {
        throw new Error('--max-prs must be a positive integer');
      }
      options.maxPrs = parsed;
    } else if (arg === '--no-ai') {
      options.noAi = true;
    } else if (arg === '--dry-run') {
      options.dryRun = true;
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }

  return options;
}

function runGit(args, options = {}) {
  return execFileSync('git', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', options.allowFailure ? 'pipe' : 'inherit'],
  }).trim();
}

function runGh(args, options = {}) {
  return execFileSync('gh', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', options.allowFailure ? 'pipe' : 'inherit'],
  }).trim();
}

export function parseGitHubRepoFromRemote(remoteUrl) {
  const cleaned = String(remoteUrl || '').trim().replace(/\.git$/, '');
  const sshMatch = cleaned.match(/github\.com[:/]([^/\s]+\/[^/\s]+)$/);
  if (sshMatch) {
    return sshMatch[1];
  }
  return null;
}

function resolveRepo(explicitRepo) {
  if (explicitRepo) {
    return explicitRepo;
  }
  for (const remote of ['upstream', 'origin']) {
    try {
      const url = runGit(['remote', 'get-url', remote], { allowFailure: true });
      const repo = parseGitHubRepoFromRemote(url);
      if (repo) {
        return repo;
      }
    } catch {
      // Try the next remote.
    }
  }
  throw new Error('Could not infer GitHub repo. Pass --repo owner/repo.');
}

function resolveEndRef(to) {
  if (to !== 'latest-tag') {
    return to;
  }

  const tag = runGit(['describe', '--tags', '--abbrev=0']);
  if (!tag) {
    throw new Error('Could not resolve latest tag');
  }
  return tag;
}

function resolveStartRef(repo, from) {
  if (from !== 'latest-release') {
    return from;
  }

  const tag = runGh(['release', 'view', '--repo', repo, '--json', 'tagName', '--jq', '.tagName']);
  if (!tag) {
    throw new Error(`Could not resolve latest GitHub Release tag for ${repo}`);
  }
  return tag;
}

function assertRefExists(ref, label) {
  try {
    runGit(['rev-parse', '--verify', `${ref}^{commit}`], { allowFailure: true });
  } catch {
    throw new Error(`${label} ref not found: ${ref}`);
  }
}

export function extractPullRequestNumbers(subject) {
  const matches = [...String(subject || '').matchAll(/\(#(\d+)\)/g)];
  return [...new Set(matches.map((match) => Number(match[1])).filter(Number.isInteger))];
}

export function parseGitLog(logText) {
  if (!logText.trim()) {
    return [];
  }

  return logText
    .split('\x1e')
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => {
      const [sha, subject, authorName, authorEmail, authoredAt] = entry.split('\x1f');
      const prNumbers = extractPullRequestNumbers(subject);
      return {
        sha,
        shortSha: sha.slice(0, 9),
        subject,
        authorName,
        authorEmail,
        authoredAt,
        prNumbers,
        primaryPrNumber: prNumbers.at(-1) || null,
      };
    });
}

function collectCommits(from, to) {
  const format = '%H%x1f%s%x1f%an%x1f%ae%x1f%aI%x1e';
  const output = runGit(['log', `${from}..${to}`, '--reverse', `--format=${format}`]);
  return parseGitLog(output);
}

function priorAuthorKeys(from) {
  const output = runGit(['log', from, '--format=%an%x1f%ae%x1e']);
  const keys = new Set();
  for (const entry of output.split('\x1e')) {
    const [name, email] = entry.trim().split('\x1f');
    if (name || email) {
      keys.add(authorKey({ authorName: name, authorEmail: email }));
    }
  }
  return keys;
}

function authorKey(author) {
  return `${String(author.authorName || '').toLowerCase()} <${String(author.authorEmail || '').toLowerCase()}>`;
}

function collectContributorStats(commits, priorKeys) {
  const contributors = new Map();
  for (const commit of commits) {
    const key = authorKey(commit);
    if (!contributors.has(key)) {
      contributors.set(key, {
        name: commit.authorName,
        email: commit.authorEmail,
        commits: 0,
        prs: new Set(),
        isNew: !priorKeys.has(key),
      });
    }
    const contributor = contributors.get(key);
    contributor.commits += 1;
    if (commit.primaryPrNumber) {
      contributor.prs.add(commit.primaryPrNumber);
    }
  }

  return [...contributors.values()]
    .map((contributor) => ({
      ...contributor,
      prs: [...contributor.prs].sort((a, b) => a - b),
    }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

function collectPrCommits(commits) {
  const byPr = new Map();
  for (const commit of commits) {
    if (!commit.primaryPrNumber) {
      continue;
    }
    const number = commit.primaryPrNumber;
    if (!byPr.has(number)) {
      byPr.set(number, []);
    }
    byPr.get(number).push(commit);
  }
  return byPr;
}

function fetchPullRequest(repo, number) {
  try {
    const json = runGh(
      [
        'pr',
        'view',
        String(number),
        '--repo',
        repo,
        '--json',
        'number,title,url,author,mergedAt,body,labels',
      ],
      { allowFailure: true },
    );
    return JSON.parse(json);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      number,
      title: null,
      url: `https://github.com/${repo}/pull/${number}`,
      author: null,
      mergedAt: null,
      body: null,
      labels: [],
      warning: `Failed to fetch PR metadata with gh: ${message}`,
    };
  }
}

function collectPullRequests(repo, commits, maxPrs) {
  const prCommits = collectPrCommits(commits);
  const numbers = [...prCommits.keys()].sort((a, b) => a - b);
  if (numbers.length > maxPrs) {
    throw new Error(`Range contains ${numbers.length} PRs, above --max-prs ${maxPrs}`);
  }

  return numbers.map((number) => {
    const detail = fetchPullRequest(repo, number);
    const commitsForPr = prCommits.get(number) || [];
    const fallbackTitle = commitsForPr.at(-1)?.subject.replace(/(?:\s+\(#\d+\))+\s*$/g, '') || `PR #${number}`;
    return {
      number,
      title: detail.title || fallbackTitle,
      url: detail.url || `https://github.com/${repo}/pull/${number}`,
      author: detail.author?.login || commitsForPr.at(-1)?.authorName || null,
      mergedAt: detail.mergedAt || commitsForPr.at(-1)?.authoredAt || null,
      labels: (detail.labels || []).map((label) => label.name || label).filter(Boolean),
      body: trimBody(detail.body),
      commits: commitsForPr.map((commit) => ({
        sha: commit.shortSha,
        subject: commit.subject,
        authorName: commit.authorName,
      })),
      warning: detail.warning,
    };
  });
}

function trimBody(body) {
  if (!body || typeof body !== 'string') {
    return '';
  }
  return body
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/\r\n/g, '\n')
    .trim()
    .slice(0, MAX_PR_BODY_CHARS);
}

function releaseTitle(from, to, resolvedTo) {
  const toLabel = to === 'latest-tag' ? resolvedTo : to;
  return `${from} to ${toLabel}`;
}

export function buildReleasePayload({ from, to, resolvedTo, repo, commits, pullRequests, contributors }) {
  return {
    repo,
    range: {
      from,
      to,
      resolvedTo,
      compareUrl: `https://github.com/${repo}/compare/${from}...${resolvedTo}`,
    },
    totals: {
      commits: commits.length,
      pullRequests: pullRequests.length,
      contributors: contributors.length,
      newContributors: contributors.filter((contributor) => contributor.isNew).length,
    },
    contributors: contributors.map((contributor) => ({
      name: contributor.name,
      commits: contributor.commits,
      prs: contributor.prs,
      isNew: contributor.isNew,
    })),
    pullRequests,
    uncategorizedCommits: commits
      .filter((commit) => commit.prNumbers.length === 0)
      .map((commit) => ({
        sha: commit.shortSha,
        subject: commit.subject,
        authorName: commit.authorName,
      })),
  };
}

export function buildOpenAiRequest({ model, title, payload }) {
  const compactPayload = JSON.stringify(payload);
  if (compactPayload.length > MAX_PROMPT_CHARS) {
    throw new Error(
      `Release payload is ${compactPayload.length} characters, above ${MAX_PROMPT_CHARS}. Use a narrower range.`,
    );
  }

  return {
    model,
    input: [
      {
        role: 'developer',
        content:
          'You write polished but concrete desktop-app release notes. Keep the tone warm, crisp, factual, and celebratory. Use tasteful excitement emojis in headings, but do not overdo them. Never invent changes not present in the input. Preserve every PR link somewhere in the output.',
      },
      {
        role: 'user',
        content: `Create Markdown release notes for "${title}" from this JSON payload.

Required structure:
- Start with a short, exciting H1 title that summarizes the whole release theme, like "# The Memory Upgrade" or "# The Hands-Free Intelligence Upgrade". Do not use the tag range as the title.
- Follow the title with a short celebratory paragraph and one tasteful emoji.
- Add "## Highlights" with multiple high-level highlight subsections, such as "### Voice & hands-free control". Use tasteful emojis in subsection headings.
- For each highlight subsection, write one or two short paragraphs maximum. Do not use highlight bullets.
- Each highlight subsection should summarize a cluster of related work, then end with compact PR links and contributor thanks, for example "([#123](url), [#124](url)) — Thank you @alice and @bob!"
- Across the highlight subsections, mention every PR in the payload exactly once if possible.
- Add "## New Contributors" celebrating first-time contributors only when there are first-time contributors. If none, omit this section entirely.
- For each new contributor, thank them and briefly describe what they contributed based on their PR titles.
- End the New Contributors section with a short note hoping they join Discord for exclusive roles and contributor rewards.
- Add "## Contributor Credits" thanking all contributors.
- Do not add a "## Pull Requests" section.
- Add "## Full Compare" with the compare URL.

JSON payload:
${compactPayload}`,
      },
    ],
  };
}

function getOpenAiKey() {
  if (process.env.OPENAI_API_KEY) {
    return process.env.OPENAI_API_KEY;
  }
  if (process.env.OPENAI_API) {
    console.error('[release-notes] Using OPENAI_API; prefer OPENAI_API_KEY for consistency with OpenAI tooling.');
    return process.env.OPENAI_API;
  }
  return null;
}

function extractResponseText(responseJson) {
  if (typeof responseJson.output_text === 'string') {
    return responseJson.output_text;
  }

  const parts = [];
  for (const item of responseJson.output || []) {
    for (const content of item.content || []) {
      if (typeof content.text === 'string') {
        parts.push(content.text);
      }
    }
  }
  return parts.join('\n').trim();
}

async function summarizeWithOpenAi(request) {
  const key = getOpenAiKey();
  if (!key) {
    throw new Error('OPENAI_API_KEY is required unless --no-ai or --dry-run is used');
  }

  const timeoutMs = 300_000;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  let response;
  try {
    response = await fetch('https://api.openai.com/v1/responses', {
      method: 'POST',
      signal: controller.signal,
      headers: {
        Authorization: `Bearer ${key}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(request),
    });
  } catch (error) {
    if (error?.name === 'AbortError') {
      throw new Error(`OpenAI Responses API timed out after ${timeoutMs}ms`);
    }
    throw error;
  } finally {
    clearTimeout(timeout);
  }

  const body = await response.text();
  let json = null;
  try {
    json = JSON.parse(body);
  } catch {
    // Keep the raw body in the error below.
  }

  if (!response.ok) {
    const message = json?.error?.message || body;
    throw new Error(`OpenAI Responses API failed (${response.status}): ${message}`);
  }

  const text = extractResponseText(json);
  if (!text) {
    throw new Error('OpenAI response did not contain output text');
  }
  return text;
}

export function renderDeterministicNotes({ title, payload }) {
  const newContributors = payload.contributors.filter((contributor) => contributor.isNew);
  const newContributorsSection = newContributors.length
    ? `
## New Contributors 🌟

${newContributors.map((contributor) => renderNewContributorLine(contributor, payload.pullRequests)).join('\n')}

Hope you join the Discord for exclusive roles and contributor rewards!
`
    : '';
  const contributorLinks = payload.contributors.map((contributor) => {
    const prList = contributor.prs.map((number) => `#${number}`).join(', ');
    return `- ${contributor.name}${prList ? ` (${prList})` : ''}`;
  });
  const highlightSections = groupPullRequestsByHighlight(payload.pullRequests).map(({ title, summary, pullRequests }) => {
    const links = pullRequests.map((pr) => `[#${pr.number}](${pr.url})`).join(', ');
    const authors = [...new Set(pullRequests.map((pr) => pr.author).filter(Boolean))];
    const thanks = authors.length ? `Thank you ${formatHandleList(authors)}!` : 'Thank you to everyone who contributed!';
    return `### ${title}\n\n${summary} (${links}) — ${thanks}`;
  });

  return `# The Intelligence Upgrade 🎉

${title} brings ${payload.totals.pullRequests} PRs across ${payload.totals.commits} commits, with upgrades across memory, voice, agents, reliability, and developer foundations. Thank you to everyone who contributed to this release. ✨

## Highlights 🚀

${highlightSections.join('\n\n') || 'No PRs found in this range.'}
${newContributorsSection}

## Contributor Credits 🙌

${contributorLinks.map((line) => `${line} — thank you!`).join('\n') || '- No contributors found.'}

## Full Compare 🧭

${payload.range.compareUrl}
`;
}

function formatHandleList(handles) {
  const names = handles.map((handle) => `@${handle}`);
  if (names.length <= 2) {
    return names.join(' and ');
  }
  return `${names.slice(0, -1).join(', ')}, and ${names.at(-1)}`;
}

function groupPullRequestsByHighlight(pullRequests) {
  const groups = [
    {
      title: '🎙️ Voice & hands-free control',
      keywords: ['voice', 'push-to-talk', 'ptt', 'wake word', 'notch', 'vad', 'stt', 'desktop control', 'automate', 'accessibility', 'vision'],
      summary:
        'Voice and hands-free control get more useful across the desktop, with always-on listening, global push-to-talk, faster command routing, and stronger automation paths for interacting with apps.',
      pullRequests: [],
    },
    {
      title: '🧠 Memory & intelligence',
      keywords: ['memory', 'embed', 'embedding', 'notion', 'sync', 'source', 'intelligence', 'vault'],
      summary:
        'Memory and intelligence flows are faster, clearer, and more resilient, from source sync controls and richer ingest to batched embeddings and delegated memory retrieval.',
      pullRequests: [],
    },
    {
      title: '✅ Tasks, chat & agent workflows',
      keywords: ['task', 'chat', 'thread', 'agent', 'subagent', 'run', 'council', 'workflow', 'skill', 'skills'],
      summary:
        'Task, chat, and agent workflows become easier to steer and inspect, with better task boards, persistent run state, safer delegation, and clearer model/thread controls.',
      pullRequests: [],
    },
    {
      title: '🔐 Reliability, security & platform polish',
      keywords: ['keyring', 'security', 'sandbox', 'action_dir', 'auth', 'session', 'cron', 'cef', 'windows', 'release', 'sentry', 'credential'],
      summary:
        'Reliability and platform behavior are tightened across security policy, credential handling, session health, CEF startup, Windows filesystem edge cases, and release-only build paths.',
      pullRequests: [],
    },
    {
      title: '🧩 Integrations, UI & developer foundations',
      keywords: ['integration', 'composio', 'mcp', 'meeting', 'ui', 'onboarding', 'font', 'docs', 'readme', 'refactor', 'module', 'artifact', 'presentation'],
      summary:
        'Integrations, UI polish, and developer foundations also move forward, including clearer connected-account surfaces, meeting-agent wiring, presentation/artifact improvements, and cleaner docs/code organization.',
      pullRequests: [],
    },
  ];

  for (const pr of pullRequests) {
    const haystack = `${pr.title} ${(pr.labels || []).join(' ')}`.toLowerCase();
    const group = groups.find((candidate) => candidate.keywords.some((keyword) => haystack.includes(keyword)));
    (group || groups.at(-1)).pullRequests.push(pr);
  }

  return groups.filter((group) => group.pullRequests.length > 0);
}

function renderNewContributorLine(contributor, pullRequests) {
  const contributed = pullRequests
    .filter((pr) => contributor.prs.includes(pr.number))
    .map((pr) => `[#${pr.number}](${pr.url}) ${pr.title}`);
  const brief = contributed.length ? ` for ${contributed.join('; ')}` : '';
  return `- Welcome ${contributor.name}! Thank you${brief}.`;
}

export function ensureAllPullRequestsLinked(markdown, pullRequests) {
  const missing = pullRequests.filter((pr) => {
    const byNumber = new RegExp(`\\[#${pr.number}\\]\\(`).test(markdown);
    const byUrl = new RegExp(`${escapeRegExp(pr.url)}(?:\\)|\\s|$)`).test(markdown);
    return !byNumber && !byUrl;
  });
  if (missing.length === 0) {
    return markdown;
  }

  const links = missing.map((pr) => `[#${pr.number}](${pr.url})`).join(', ');
  const authors = [...new Set(missing.map((pr) => pr.author).filter(Boolean))];
  const thanks = authors.length ? `Thank you ${formatHandleList(authors)}!` : 'Thank you to everyone who contributed!';
  const insertion = `\n### Additional highlights 🔗\n\nA few more focused contributions round out this release with targeted fixes and improvements that are worth calling out. (${links}) — ${thanks}\n`;

  const sectionMatch = markdown.match(/\n## (New Contributors|Contributor Credits|Full Compare)\b/);
  if (!sectionMatch || typeof sectionMatch.index !== 'number') {
    return `${markdown.trim()}${insertion}`;
  }

  return `${markdown.slice(0, sectionMatch.index).trimEnd()}${insertion}${markdown.slice(sectionMatch.index)}`;
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

async function main() {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(`[release-notes] ${error.message}`);
    console.error(usage());
    process.exit(2);
  }

  if (options.help) {
    process.stdout.write(usage());
    return;
  }

  const repo = resolveRepo(options.repo);
  const from = resolveStartRef(repo, options.from);
  const resolvedTo = resolveEndRef(options.to);

  assertRefExists(from, 'Start');
  assertRefExists(resolvedTo, 'End');

  console.error(`[release-notes] Collecting ${repo} changes from ${from} to ${resolvedTo}`);
  const commits = collectCommits(from, resolvedTo);
  const contributors = collectContributorStats(commits, priorAuthorKeys(from));
  const pullRequests = collectPullRequests(repo, commits, options.maxPrs);
  const payload = buildReleasePayload({
    from,
    to: options.to,
    resolvedTo,
    repo,
    commits,
    pullRequests,
    contributors,
  });
  const title = releaseTitle(from, options.to, resolvedTo);

  let markdown;
  if (options.dryRun) {
    const request = buildOpenAiRequest({ model: options.model, title, payload });
    markdown = JSON.stringify(request, null, 2);
  } else if (options.noAi) {
    markdown = renderDeterministicNotes({ title, payload });
  } else {
    const request = buildOpenAiRequest({ model: options.model, title, payload });
    markdown = await summarizeWithOpenAi(request);
    markdown = ensureAllPullRequestsLinked(markdown, payload.pullRequests);
  }

  if (options.output) {
    const outputPath = resolve(options.output);
    if (existsSync(outputPath) && basename(outputPath).startsWith('.')) {
      throw new Error(`Refusing to overwrite hidden file: ${outputPath}`);
    }
    writeFileSync(outputPath, markdown.endsWith('\n') ? markdown : `${markdown}\n`);
    console.error(`[release-notes] Wrote ${outputPath}`);
  } else {
    process.stdout.write(markdown.endsWith('\n') ? markdown : `${markdown}\n`);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(`[release-notes] ${error.message}`);
    process.exit(1);
  });
}
