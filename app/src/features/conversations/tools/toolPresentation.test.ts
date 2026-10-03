import { describe, expect, it } from 'vitest';

import {
  describeToolCall,
  type DescribeToolCallInput,
  sentenceCase,
  toolLabel,
} from './toolPresentation';

const label = (input: DescribeToolCallInput) => toolLabel(describeToolCall(input));
const done = (name: string, args?: unknown, extra: Partial<DescribeToolCallInput> = {}) =>
  label({ name, args, status: 'success', ...extra });
const active = (name: string, args?: unknown, extra: Partial<DescribeToolCallInput> = {}) =>
  label({ name, args, status: 'running', ...extra });

/**
 * One test per mislabelling that shipped. Each names the bug it pins so a
 * regression reads as that bug coming back, not as a changed string.
 */
describe('tool labels: regressions', () => {
  it('does not call non-web searches "Searched the web" (tool_search, memory, skills)', () => {
    for (const name of [
      'tool_search',
      'memory_hybrid_search',
      'memory_vector_search',
      'skill_registry_search',
      'mcp_registry_search',
      'search_tool_catalog',
      'gitbooks_search',
    ]) {
      expect(done(name, { query: 'x' }), name).not.toMatch(/web/i);
    }
  });

  it('does not call a Composio fetch with a query argument a web search', () => {
    expect(done('GMAIL_FETCH_EMAILS', { query: 'from:boss' })).toBe('Used Gmail');
    expect(describeToolCall({ name: 'GMAIL_FETCH_EMAILS' }).chip).toBe('Fetch emails');
  });

  it('does not call any tool with a url argument "Fetched from the web"', () => {
    expect(done('storage_get_link', { url: 'https://x.dev' })).toBe('Created share link');
    expect(done('gitbooks_get_page', { url: 'https://docs.x' })).toBe('Read docs');
    expect(done('install_workflow_from_url', { url: 'https://x' })).toBe('Installed skill');
  });

  it('labels the docs search as a docs search', () => {
    expect(active('gitbooks_search', { query: 'install' })).toBe('Searching docs');
  });

  it('never shouts a Composio action slug', () => {
    expect(done('GMAIL_SEND_EMAIL')).toBe('Used Gmail');
    expect(describeToolCall({ name: 'GMAIL_SEND_EMAIL' }).chip).toBe('Send email');
    expect(done('OUTLOOK_SEND_EMAIL')).toBe('Used Outlook');
    expect(done('GOOGLECALENDAR_CREATE_EVENT')).toBe('Used Google Calendar');
    expect(describeToolCall({ name: 'GOOGLECALENDAR_CREATE_EVENT' }).chip).toBe('Create event');
    // An unknown toolkit still reads as words, not a slug.
    expect(done('ACMECORP_SYNC_ALL_RECORDS')).toBe('Used Acmecorp');
    expect(describeToolCall({ name: 'ACMECORP_SYNC_ALL_RECORDS' }).chip).toBe('Sync all records');
  });

  it('names the MCP tool and server instead of "Calling MCP tool"', () => {
    const p = describeToolCall({
      name: 'mcp_call_tool',
      args: { server: 'linear', tool: 'create_issue' },
      status: 'success',
    });
    expect(toolLabel(p)).toBe('Called create_issue');
    expect(p.chip).toBe('linear');
    expect(active('mcp_registry_tool_call', { server_id: 'gh', tool_name: 'list_prs' })).toBe(
      'Calling list_prs'
    );
  });

  it('uses the past tense once a call settles', () => {
    expect(active('file_read', { path: 'a.ts' })).toBe('Reading file');
    expect(done('file_read', { path: 'a.ts' })).toBe('Read file');
    expect(label({ name: 'file_read', status: 'error' })).toBe('Read file');
    expect(active('web_search_tool')).toBe('Searching the web');
    expect(done('web_search_tool')).toBe('Searched the web');
  });

  it('labels the search tool the core actually streams, not only its settings id', () => {
    expect(done('web_search_tool', { query: 'rust' })).toBe('Searched the web');
    expect(describeToolCall({ name: 'web_search_tool', args: { query: 'rust' } }).chip).toBe(
      'rust'
    );
  });

  it('covers every bring-your-own-key search engine', () => {
    for (const name of [
      'exa_search',
      'tavily_search',
      'querit_search',
      'parallel_search',
      'tinyfish_search',
    ]) {
      expect(done(name), name).toBe('Searched the web');
      expect(describeToolCall({ name }).body, name).toBe('webSearch');
    }
    expect(done('brave_news_search')).toBe('Searched news');
    expect(done('brave_image_search')).toBe('Searched images');
  });

  it('covers the search role tools and provider answer tools', () => {
    expect(done('web_answer_tool', { query: 'q' })).toBe('Asked the web');
    expect(describeToolCall({ name: 'web_answer_tool', args: { query: 'q' } }).chip).toBe('q');
    for (const name of ['web_answer_tool', 'gemini_agentic_search', 'exa_answer']) {
      expect(describeToolCall({ name }).body, name).toBe('webSearch');
    }
    expect(done('gemini_deep_research')).toBe('Researched');
    expect(describeToolCall({ name: 'gemini_deep_research' }).body).toBe('webSearch');
    expect(done('web_contents_tool', { urls: ['https://a.dev/x', 'https://b.dev'] })).toBe(
      'Read pages'
    );
    expect(
      describeToolCall({ name: 'web_contents_tool', args: { urls: ['https://a.dev/x'] } }).body
    ).toBe('webFetch');
    // Parallel was removed from the core, but old transcripts still render.
    expect(done('parallel_research')).toBe('Researched');
  });

  it('describes the deferred-tool bridge as the tool it calls', () => {
    expect(done('tool_call', { name: 'SLACK_SEND_MESSAGE', arguments: {} })).toBe('Used Slack');
    expect(done('tool_call', { name: 'file_read', arguments: { path: '/a/b/c/d.ts' } })).toBe(
      'Read file'
    );
  });

  it('switches collapsed tools on their action argument', () => {
    expect(done('memory', { action: 'recall', query: 'x' })).toBe('Recalled memories');
    expect(done('memory', { action: 'store', key: 'k' })).toBe('Saved to memory');
    expect(done('cron', { action: 'add', name: 'daily' })).toBe('Scheduled task');
    expect(done('browser', { action: 'click', selector: '#go' })).toBe('Clicked');
  });

  it('labels the memory v2 actions and chips their main argument', () => {
    const recall = describeToolCall({
      name: 'memory',
      args: { action: 'recall', question: 'what do I prefer?' },
    });
    expect(recall.chip).toBe('what do I prefer?');
    expect(done('memory', { action: 'fetch', query: 'tea' })).toBe(done('memory_hybrid_search'));
    expect(describeToolCall({ name: 'memory', args: { action: 'fetch', query: 'tea' } }).chip).toBe(
      'tea'
    );
    expect(done('memory', { action: 'learn', text: 'likes tea' })).toBe('Saved to memory');
    expect(
      describeToolCall({ name: 'memory', args: { action: 'learn', text: 'likes tea' } }).chip
    ).toBe('likes tea');
  });

  it('labels named agents and delegations by what they do', () => {
    expect(done('subagent:planner')).toBe('Planned next steps');
    expect(done('spawn_subagent', { agent_id: 'critic' })).toBe('Reviewed the work');
    expect(done('delegate_gmail')).toBe('Used Gmail');
    expect(active('run_code')).toBe('Running code');
    expect(
      done('spawn_subagent', { agent_id: 'integrations_agent', toolkit: 'notion', prompt: 'p' })
    ).toBe('Used Notion');
  });

  it('prefers the server label only for tools it cannot describe', () => {
    // A known tool ignores the core's humanized label.
    expect(done('file_read', {}, { serverLabel: 'File Read' })).toBe('Read file');
    // An unknown tool takes a readable server label.
    expect(done('frobnicate', {}, { serverLabel: 'Frobnicated the widget' })).toBe(
      'Frobnicated the widget'
    );
    // A server label that is itself a leaked identifier is not trusted.
    expect(done('frobnicate', {}, { serverLabel: 'FROB NICATE' })).toBe('Used frobnicate');
    expect(done('frobnicate', {}, { serverLabel: 'frob_nicate' })).toBe('Used frobnicate');
  });

  it('falls back to a sentence-cased, tense-aware label', () => {
    expect(active('some_new_tool')).toBe('Using some new tool');
    expect(done('some_new_tool')).toBe('Used some new tool');
    expect(sentenceCase('GMAIL_SEND_EMAIL')).toBe('Gmail send email');
  });

  it('renders a degraded placeholder name without shouting', () => {
    expect(done('tool')).toBe('Used tool');
    expect(done('')).toBe('Used tool');
  });
});

describe('tool chips', () => {
  it('shortens paths and urls', () => {
    expect(describeToolCall({ name: 'file_read', args: { path: '/a/b/c/d.ts' } }).chip).toBe(
      '…/c/d.ts'
    );
    expect(
      describeToolCall({ name: 'web_fetch', args: { url: 'https://docs.rs/tokio/latest' } }).chip
    ).toBe('docs.rs/tokio/latest');
  });

  it('reads args from a JSON buffer', () => {
    expect(describeToolCall({ name: 'shell', args: '{"command":"ls -la"}' }).chip).toBe('ls -la');
  });

  it('caps a chip so model text cannot blow up a row', () => {
    const chip = describeToolCall({ name: 'grep', args: { pattern: 'x'.repeat(500) } }).chip;
    expect(chip?.length).toBeLessThanOrEqual(80);
  });
});

describe('tool labels: translation', () => {
  it('serves the label through the phrase key and fills placeholders', () => {
    const t = (key: string, fallback?: string) =>
      key === 'conversations.tools.useApp.done' ? '{app} benutzt' : (fallback ?? key);
    expect(toolLabel(describeToolCall({ name: 'GMAIL_SEND_EMAIL', status: 'success' }), t)).toBe(
      'Gmail benutzt'
    );
  });
});
