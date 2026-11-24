/**
 * Turn a web-search tool result into rows the search element can render.
 *
 * Three inputs, most trustworthy first:
 *
 * 1. The structured payload a current core attaches to `tool_result`
 *    (`{ kind: "web_search", query, provider, role?, answer?, citations?,
 *    fallback_from?, in_progress?, results: [...] }`).
 * 2. The plain-text rendering every role tool returns to the model:
 *
 *        Search results for: <query> (via <Provider>, after <Skipped>)
 *        1. <title>
 *           <url>
 *           Published: <date>
 *           <excerpt>
 *
 *    `Answer for:` puts the answer text under the heading and lists its
 *    citations after a `Sources:` line (`[1] <title> — <url>`);
 *    `Page contents for:` and `Research still running for:` share the shape.
 * 3. The markdown rendering (`## [title](url)` / `> excerpt`, citations under
 *    `### Sources`) used when the core prefers markdown.
 *
 * Every URL is model- or provider-supplied, so only well-formed `http(s)`
 * URLs are admitted; anything else is dropped rather than rendered as a
 * link.
 */
import { truncateText } from '../../../utils/truncateText';

/** Upper bound on a provider label, so a malformed marker can't blow up a row. */
const MAX_SEARCH_PROVIDER_LENGTH = 32;

/**
 * Extract the resolved search provider from a completed web-search result.
 * Every search engine tags its output with a `(via <Provider>)` marker on the
 * heading line (managed resolves to "Exa" by default, or to whatever the
 * backend reports; BYOK engines tag "Brave"/"Querit"/"Seltz"/"Tavily"). Reading it back
 * keeps the attribution dynamic: it is driven by what actually ran, never by
 * a hardcoded provider name (#5136).
 *
 * Only the first line is inspected, and only its *trailing* marker, so neither
 * a `(via …)` string inside a result excerpt nor one inside the echoed query
 * (`Search results for: login (via OAuth) (via Exa)`) can be mistaken for the
 * provider. Returns `undefined` while the call is still running (no result
 * yet) or if no marker is present.
 */
export function extractSearchProvider(result: string | undefined): string | undefined {
  return viaMarker(result)?.provider;
}

/**
 * Providers the role tool skipped before one answered, read from the
 * `(via Gemini, after Exa, Tavily)` heading marker. Empty when the first
 * provider answered or there is no marker.
 */
export function extractSearchFallbacks(result: string | undefined): string[] {
  return viaMarker(result)?.fallbackFrom ?? [];
}

function viaMarker(
  result: string | undefined
): { provider: string; fallbackFrom: string[] } | undefined {
  if (!result) return undefined;
  const headingLine = result.split('\n', 1)[0];
  const marker = headingLine?.match(/\(via ([^)]+)\)\s*_?$/i)?.[1]?.trim();
  if (!marker) return undefined;
  const [head, tail] = marker.split(/,\s*after\s+/i, 2);
  const provider = head.trim();
  if (!provider || provider.length > MAX_SEARCH_PROVIDER_LENGTH) return undefined;
  const fallbackFrom = (tail ?? '')
    .split(/,\s*/)
    .map(label => label.trim())
    .filter(label => label && label.length <= MAX_SEARCH_PROVIDER_LENGTH);
  return { provider, fallbackFrom };
}

/**
 * True when a failed search call ran out of TinyHumans balance. The core's
 * error text for that case contains "balance is too low"; the value may be the
 * raw error string or any object that carries it.
 */
export function isSearchBalanceError(value: unknown): boolean {
  if (value === undefined || value === null) return false;
  let text: string;
  if (typeof value === 'string') text = value;
  else {
    try {
      text = JSON.stringify(value);
    } catch {
      return false;
    }
  }
  return /balance is too low/i.test(text);
}

export interface WebSearchHit {
  title: string;
  url: string;
  domain: string;
  published?: string;
  excerpt?: string;
}

export type WebSearchRole = 'search' | 'answer' | 'contents';

export interface ParsedWebSearch {
  query?: string;
  provider?: string;
  results: WebSearchHit[];
  /** The call completed and found nothing (distinct from "not parseable"). */
  empty: boolean;
  /** Which role tool produced the result, when the core said. */
  role?: WebSearchRole;
  /** Grounded answer text (answer role). */
  answer?: string;
  /** Sources the answer cites. Only present when there are some. */
  citations?: WebSearchHit[];
  /** Providers tried and skipped before `provider` answered. */
  fallbackFrom?: string[];
  /** A deep-research job that has not finished yet. */
  inProgress?: boolean;
}

const MAX_EXCERPT = 280;
/** Cap on the answer text the chat card shows; the model still gets it all. */
const MAX_ANSWER = 4000;
const ROLES: readonly WebSearchRole[] = ['search', 'answer', 'contents'];

function safeHttpUrl(value: string): URL | undefined {
  try {
    const url = new URL(value.trim());
    return url.protocol === 'http:' || url.protocol === 'https:' ? url : undefined;
  } catch {
    return undefined;
  }
}

function clip(text: string | undefined, max = MAX_EXCERPT): string | undefined {
  const cleaned = text?.replace(/\s+/g, ' ').trim();
  if (!cleaned) return undefined;
  return truncateText(cleaned, max);
}

function hit(
  title: string | undefined,
  rawUrl: string | undefined,
  published?: string,
  excerpt?: string
): WebSearchHit | undefined {
  const url = rawUrl ? safeHttpUrl(rawUrl) : undefined;
  if (!url) return undefined;
  const domain = url.hostname.replace(/^www\./, '');
  return {
    title: clip(title, 160) ?? domain,
    url: url.toString(),
    domain,
    ...(clip(published, 40) ? { published: clip(published, 40) } : {}),
    ...(clip(excerpt) ? { excerpt: clip(excerpt) } : {}),
  };
}

function fromStructured(value: unknown): ParsedWebSearch | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const payload = value as Record<string, unknown>;
  if (payload.kind !== 'web_search' || !Array.isArray(payload.results)) return undefined;
  const results = payload.results
    .map(item => {
      if (!item || typeof item !== 'object') return undefined;
      const row = item as Record<string, unknown>;
      const str = (key: string) =>
        typeof row[key] === 'string' ? (row[key] as string) : undefined;
      return hit(str('title'), str('url'), str('published'), str('excerpt'));
    })
    .filter((row): row is WebSearchHit => row !== undefined);
  const citations = Array.isArray(payload.citations)
    ? payload.citations
        .map(item => {
          if (!item || typeof item !== 'object') return undefined;
          const row = item as Record<string, unknown>;
          return hit(
            typeof row.title === 'string' ? row.title : undefined,
            typeof row.url === 'string' ? row.url : undefined
          );
        })
        .filter((row): row is WebSearchHit => row !== undefined)
    : [];
  const fallbackFrom = Array.isArray(payload.fallback_from)
    ? payload.fallback_from.filter(
        (label): label is string =>
          typeof label === 'string' &&
          label.trim().length > 0 &&
          label.length <= MAX_SEARCH_PROVIDER_LENGTH
      )
    : [];
  const answer = typeof payload.answer === 'string' ? clipAnswer(payload.answer) : undefined;
  const role = ROLES.find(r => r === payload.role);
  return withExtras(
    {
      query: typeof payload.query === 'string' ? payload.query : undefined,
      provider: typeof payload.provider === 'string' ? payload.provider : undefined,
      results,
      empty: results.length === 0 && citations.length === 0 && !answer,
    },
    { role, answer, citations, fallbackFrom, inProgress: payload.in_progress === true }
  );
}

function clipAnswer(text: string): string | undefined {
  const trimmed = text.trim();
  if (!trimmed) return undefined;
  return truncateText(trimmed, MAX_ANSWER);
}

/** Attach the optional answer-side fields only when they carry something. */
function withExtras(
  base: ParsedWebSearch,
  extras: {
    role?: WebSearchRole;
    answer?: string;
    citations?: WebSearchHit[];
    fallbackFrom?: string[];
    inProgress?: boolean;
  }
): ParsedWebSearch {
  return {
    ...base,
    ...(extras.role ? { role: extras.role } : {}),
    ...(extras.answer ? { answer: extras.answer } : {}),
    ...(extras.citations?.length ? { citations: extras.citations } : {}),
    ...(extras.fallbackFrom?.length ? { fallbackFrom: extras.fallbackFrom } : {}),
    ...(extras.inProgress ? { inProgress: true } : {}),
  };
}

/** `[1] Title — https://x` / `[1] https://x` (plain) or `- [Title](url)` (markdown). */
function citationFromLine(line: string): WebSearchHit | undefined {
  const md = line.match(/^\s*-\s+\[(.+)\]\((\S+)\)\s*$/);
  if (md) return hit(md[1], md[2]);
  const plain = line.match(/^\s*\[\d+\]\s+(.+)$/);
  if (!plain) return undefined;
  const body = plain[1].trim();
  const split = body.lastIndexOf(' \u2014 ');
  if (split > 0) return hit(body.slice(0, split), body.slice(split + 3));
  return hit(undefined, body);
}

/** Heading kinds the core writes, most specific first. */
function headingRole(heading: string): { role?: WebSearchRole; inProgress: boolean } {
  if (/^Research still running for:/i.test(heading)) return { role: 'answer', inProgress: true };
  if (/^Answer for:/i.test(heading)) return { role: 'answer', inProgress: false };
  if (/^Page contents for:/i.test(heading)) return { role: 'contents', inProgress: false };
  return { inProgress: false };
}

/** Strip the trailing `(via X)` marker from a heading's query part. */
function headingQuery(heading: string): string | undefined {
  const query = heading.replace(/\s*\(via [^)]+\)\s*$/i, '').trim();
  return query.replace(/^`|`$/g, '').trim() || undefined;
}

function fromText(text: string): ParsedWebSearch | undefined {
  const allLines = text.split('\n');
  const heading = allLines[0]?.trim() ?? '';
  const via = viaMarker(heading);
  const provider = via?.provider;
  const fallbackFrom = via?.fallbackFrom ?? [];

  // Citations sit after a `Sources:` (plain) or `### Sources` (markdown) line.
  const sourcesAt = allLines.findIndex(
    (line, index) => index > 0 && /^(?:#{2,3}\s+)?Sources:?\s*$/i.test(line.trim())
  );
  const lines = sourcesAt > 0 ? allLines.slice(0, sourcesAt) : allLines;
  const citations =
    sourcesAt > 0
      ? allLines
          .slice(sourcesAt + 1)
          .map(citationFromLine)
          .filter((row): row is WebSearchHit => row !== undefined)
      : [];

  const emptyMatch = heading.match(/^_?No (?:\w+ )?results (?:found )?for:?\s*(.+?)_?$/i);
  if (emptyMatch) {
    return withExtras(
      {
        query: headingQuery(emptyMatch[1].replace(/_$/, '').replace(/[._]+$/, '')),
        provider,
        results: [],
        empty: true,
      },
      { fallbackFrom }
    );
  }

  // Markdown rendering.
  const mdHeading = heading.match(
    /^#\s+((?:\w+ results|Answer for|Page contents for|Research still running for)\b.*)$/i
  );
  if (mdHeading || lines.some(line => /^##\s+\[.+\]\(.+\)\s*$/.test(line))) {
    const headingText = mdHeading?.[1] ?? '';
    const { role, inProgress } = headingRole(headingText);
    const results: WebSearchHit[] = [];
    const answerLines: string[] = [];
    let current: { title: string; url: string; published?: string; excerpt: string[] } | null =
      null;
    const flush = () => {
      if (!current) return;
      const row = hit(current.title, current.url, current.published, current.excerpt.join(' '));
      if (row) results.push(row);
      current = null;
    };
    for (const line of lines.slice(1)) {
      const link = line.match(/^##\s+\[(.+)\]\((\S+)\)\s*$/);
      if (link) {
        flush();
        current = { title: link[1], url: link[2], excerpt: [] };
        continue;
      }
      if (!current) {
        if (line.trim()) answerLines.push(line.trim());
        continue;
      }
      const published = line.match(/^_Published:\s*(.+?)_\s*$/);
      if (published) current.published = published[1];
      else if (line.startsWith('>')) current.excerpt.push(line.replace(/^>\s?/, ''));
    }
    flush();
    const answer = role ? clipAnswer(answerLines.join('\n')) : undefined;
    return withExtras(
      {
        query: mdHeading ? mdHeadingQuery(headingText) : undefined,
        provider,
        results,
        empty: results.length === 0 && citations.length === 0 && !answer,
      },
      { role, answer, citations, fallbackFrom, inProgress }
    );
  }

  // Plain-text rendering.
  const textHeading = heading.match(
    /^(?:(?:Search|\w+) results for|Answer for|Page contents for|Research still running for):\s*(.+)$/i
  );
  if (!textHeading) return undefined;
  const { role, inProgress } = headingRole(heading);
  // An item is a numbered title line followed by its indented URL line. An
  // excerpt's continuation lines are not indented, so that shape is what
  // separates the next item from a wrapped excerpt.
  const isItemStart = (index: number) =>
    /^\s*\d+\.\s+\S/.test(lines[index] ?? '') && /^\s{2,}\S/.test(lines[index + 1] ?? '');
  const results: WebSearchHit[] = [];
  const answerLines: string[] = [];
  let i = 1;
  while (i < lines.length) {
    if (!isItemStart(i)) {
      // Text between the heading and the first item is the answer.
      if (results.length === 0 && lines[i].trim()) answerLines.push(lines[i].trim());
      i += 1;
      continue;
    }
    const title = lines[i].replace(/^\s*\d+\.\s+/, '');
    const urlLine = lines[i + 1].trim();
    let published: string | undefined;
    const excerpt: string[] = [];
    let j = i + 2;
    for (; j < lines.length && !isItemStart(j); j += 1) {
      const trimmed = lines[j].trim();
      const date = trimmed.match(/^Published:\s*(.+)$/);
      if (date) published = date[1];
      else if (!/^Author:/.test(trimmed) && trimmed) excerpt.push(trimmed);
    }
    const row = hit(title, urlLine, published, excerpt.join(' '));
    if (row) results.push(row);
    i = j;
  }
  const answer = role === 'answer' ? clipAnswer(answerLines.join('\n')) : undefined;
  return withExtras(
    {
      query: headingQuery(textHeading[1]),
      provider,
      results,
      empty: results.length === 0 && citations.length === 0 && !answer,
    },
    { role, answer, citations, fallbackFrom, inProgress }
  );
}

/** Query from a markdown heading: `Search results — `q` (via X)` or `Answer for: q (via X)`. */
function mdHeadingQuery(headingText: string): string | undefined {
  const match =
    headingText.match(/^\w+ results\s*(?:--|:|\u2014)\s*(.+)$/i) ??
    headingText.match(/^[\w\s]+?for:\s*(.+)$/i);
  return match ? headingQuery(match[1]) : undefined;
}

/**
 * Parse a web-search result. `structured` wins when present; otherwise the
 * text `output` is parsed. Returns `undefined` when neither is recognisable,
 * so the caller can fall back to the generic output view.
 */
export function parseWebSearchResult(
  output: unknown,
  structured?: unknown
): ParsedWebSearch | undefined {
  const fromPayload = fromStructured(structured) ?? fromStructured(output);
  if (fromPayload) return fromPayload;
  if (typeof output !== 'string' || !output.trim()) return undefined;
  return fromText(output.trim());
}
