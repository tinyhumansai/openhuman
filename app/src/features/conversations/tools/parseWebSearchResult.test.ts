import { describe, expect, it } from 'vitest';

import { extractAgentSources } from '../../../utils/toolTimelineFormatting';
import {
  extractSearchFallbacks,
  extractSearchProvider,
  isSearchBalanceError,
  parseWebSearchResult,
} from './parseWebSearchResult';

const TEXT = [
  'Search results for: rust async traits (via Exa)',
  '1. Async fn in traits are now stable',
  '   https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits.html',
  '   Published: 2023-12-21',
  '   Rust 1.75 stabilizes async fn in traits.',
  'This line wraps from the excerpt.',
  '2. javascript link',
  '   javascript:alert(1)',
  '3. Tokio tutorial',
  '   https://tokio.rs/tokio/tutorial',
  '   Learn async Rust.',
].join('\n');

describe('parseWebSearchResult', () => {
  it('parses the plain-text rendering every engine returns', () => {
    const parsed = parseWebSearchResult(TEXT);
    expect(parsed?.query).toBe('rust async traits');
    expect(parsed?.provider).toBe('Exa');
    expect(parsed?.results.map(r => r.domain)).toEqual(['blog.rust-lang.org', 'tokio.rs']);
    expect(parsed?.results[0]).toMatchObject({
      title: 'Async fn in traits are now stable',
      published: '2023-12-21',
      excerpt: 'Rust 1.75 stabilizes async fn in traits. This line wraps from the excerpt.',
    });
  });

  it('drops non-http(s) urls instead of rendering them as links', () => {
    const urls = parseWebSearchResult(TEXT)?.results.map(r => r.url) ?? [];
    expect(urls.every(url => url.startsWith('https://'))).toBe(true);
  });

  it('reports an empty search as empty, not unparseable', () => {
    expect(parseWebSearchResult('No results found for: zzqx (via Brave)')).toEqual({
      query: 'zzqx',
      provider: 'Brave',
      results: [],
      empty: true,
    });
  });

  it('keeps a "(via …)" inside the query out of the provider', () => {
    const parsed = parseWebSearchResult('Search results for: login (via OAuth) (via Exa)');
    expect(parsed?.provider).toBe('Exa');
    expect(parsed?.query).toBe('login (via OAuth)');
    expect(extractSearchProvider('Search results for: login (via OAuth) (via Exa)')).toBe('Exa');
  });

  it('parses the markdown rendering', () => {
    const md = [
      '# Search results — `vite plugins` (via Tavily)',
      '',
      '## [Vite plugin API](https://vite.dev/guide/api-plugin)',
      '_Published: 2025-01-01_',
      '',
      '> Plugins extend Vite.',
    ].join('\n');
    const parsed = parseWebSearchResult(md);
    expect(parsed?.provider).toBe('Tavily');
    expect(parsed?.results).toEqual([
      {
        title: 'Vite plugin API',
        url: 'https://vite.dev/guide/api-plugin',
        domain: 'vite.dev',
        published: '2025-01-01',
        excerpt: 'Plugins extend Vite.',
      },
    ]);
  });

  it('prefers the structured payload over the text', () => {
    const parsed = parseWebSearchResult('Search results for: ignored (via Exa)', {
      kind: 'web_search',
      query: 'structured',
      provider: 'Parallel',
      results: [{ title: 'A', url: 'https://www.a.dev/x', excerpt: 'e' }, { url: 'file:///etc' }],
    });
    expect(parsed).toEqual({
      query: 'structured',
      provider: 'Parallel',
      results: [{ title: 'A', url: 'https://www.a.dev/x', domain: 'a.dev', excerpt: 'e' }],
      empty: false,
    });
  });

  it('does not cut an emoji in half when clipping a title or answer', () => {
    const parsed = parseWebSearchResult('ignored', {
      kind: 'web_search',
      query: 'q',
      provider: 'Exa',
      role: 'answer',
      answer: `${'a'.repeat(3998)}😀 more`,
      results: [{ title: `${'a'.repeat(158)}😀 more`, url: 'https://a.dev/x' }],
    });
    expect(parsed?.results[0]?.title).toBe(`${'a'.repeat(158)}…`);
    expect(parsed?.answer).toBe(`${'a'.repeat(3998)}…`);
  });

  it('reads the answer, citations and fallback from the structured payload', () => {
    const parsed = parseWebSearchResult('ignored', {
      kind: 'web_search',
      query: 'q',
      provider: 'Gemini',
      role: 'answer',
      answer: '  The answer.  ',
      citations: [
        { url: 'https://a.dev/1', title: 'One' },
        { url: 'https://b.dev/2', title: null },
        { url: 'ftp://nope' },
      ],
      fallback_from: ['Exa', 42],
      results: [],
    });
    expect(parsed).toEqual({
      query: 'q',
      provider: 'Gemini',
      role: 'answer',
      answer: 'The answer.',
      citations: [
        { title: 'One', url: 'https://a.dev/1', domain: 'a.dev' },
        { title: 'b.dev', url: 'https://b.dev/2', domain: 'b.dev' },
      ],
      fallbackFrom: ['Exa'],
      results: [],
      empty: false,
    });
  });

  it('flags an unfinished deep-research payload', () => {
    const parsed = parseWebSearchResult(undefined, {
      kind: 'web_search',
      query: 'q',
      provider: 'Gemini Deep Research',
      role: 'answer',
      in_progress: true,
      results: [],
    });
    expect(parsed?.inProgress).toBe(true);
    expect(parsed?.empty).toBe(true);
  });

  it('parses the plain-text answer rendering with its sources', () => {
    const text = [
      'Answer for: who won the final (via Gemini, after Exa, Tavily)',
      '',
      'Team A won 2-1.',
      'It was close.',
      '',
      'Sources:',
      '[1] Final report \u2014 https://news.example/final',
      '[2] https://www.other.example/x',
    ].join('\n');
    const parsed = parseWebSearchResult(text);
    expect(parsed?.query).toBe('who won the final');
    expect(parsed?.provider).toBe('Gemini');
    expect(parsed?.fallbackFrom).toEqual(['Exa', 'Tavily']);
    expect(parsed?.role).toBe('answer');
    expect(parsed?.answer).toBe('Team A won 2-1.\nIt was close.');
    expect(parsed?.citations?.map(c => [c.title, c.url])).toEqual([
      ['Final report', 'https://news.example/final'],
      ['other.example', 'https://www.other.example/x'],
    ]);
    expect(parsed?.results).toEqual([]);
    expect(parsed?.empty).toBe(false);
  });

  it('parses the markdown answer rendering with its sources', () => {
    const md = [
      '# Answer for: q (via Gemini)',
      '',
      'The answer.',
      '',
      '### Sources',
      '- [Doc](https://docs.example/a)',
    ].join('\n');
    const parsed = parseWebSearchResult(md);
    expect(parsed?.query).toBe('q');
    expect(parsed?.answer).toBe('The answer.');
    expect(parsed?.citations?.[0]).toMatchObject({ title: 'Doc', url: 'https://docs.example/a' });
  });

  it('recognises contents and still-running research headings', () => {
    const contents = parseWebSearchResult(
      [
        'Page contents for: https://a.dev (via Exa)',
        '1. A page',
        '   https://a.dev/',
        '   Body',
      ].join('\n')
    );
    expect(contents?.role).toBe('contents');
    expect(contents?.results[0]?.url).toBe('https://a.dev/');
    const running = parseWebSearchResult('Research still running for: deep q (via Gemini)');
    expect(running?.inProgress).toBe(true);
    expect(running?.query).toBe('deep q');
  });

  it('splits the fallback list out of the via marker', () => {
    expect(extractSearchProvider('Answer for: q (via Gemini, after Exa)')).toBe('Gemini');
    expect(extractSearchFallbacks('Answer for: q (via Gemini, after Exa, Brave)')).toEqual([
      'Exa',
      'Brave',
    ]);
    expect(extractSearchFallbacks('Search results for: q (via Exa)')).toEqual([]);
    expect(extractSearchFallbacks(undefined)).toEqual([]);
  });

  it('detects the managed-search balance error in text or objects', () => {
    expect(isSearchBalanceError('Your balance is too low to search')).toBe(true);
    expect(isSearchBalanceError({ causePlain: 'Balance is too low.' })).toBe(true);
    expect(isSearchBalanceError('timed out')).toBe(false);
    expect(isSearchBalanceError(undefined)).toBe(false);
  });

  it('returns undefined for output it does not recognise', () => {
    expect(parseWebSearchResult('some other text')).toBeUndefined();
    expect(parseWebSearchResult(undefined)).toBeUndefined();
  });
});

describe('extractAgentSources', () => {
  it('lists the hits of a completed web search as sources', () => {
    const sources = extractAgentSources([
      {
        id: 's1',
        name: 'web_search_tool',
        round: 1,
        seq: 0,
        status: 'success',
        argsBuffer: '{"query":"rust async traits"}',
        result: TEXT,
      },
      {
        id: 'f1',
        name: 'web_fetch',
        round: 1,
        seq: 1,
        status: 'success',
        argsBuffer: '{"url":"https://tokio.rs/tokio/tutorial"}',
      },
    ]);
    expect(sources.map(s => s.url)).toEqual([
      'https://blog.rust-lang.org/2023/12/21/async-fn-rpit-in-traits.html',
      'https://tokio.rs/tokio/tutorial',
    ]);
    expect(sources[0].title).toBe('Async fn in traits are now stable');
  });

  it("lists an answer call's citations as sources", () => {
    const sources = extractAgentSources([
      {
        id: 'a1',
        name: 'web_answer_tool',
        round: 1,
        seq: 0,
        status: 'success',
        argsBuffer: '{"query":"q"}',
        result:
          'Answer for: q (via Gemini)\n\nYes.\n\nSources:\n[1] Doc \u2014 https://docs.example/a',
      },
    ]);
    expect(sources.map(s => s.url)).toEqual(['https://docs.example/a']);
  });
});
