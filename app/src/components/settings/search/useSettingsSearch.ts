// ---------------------------------------------------------------------------
// useSettingsSearch
//
// Filters the settings search registry against a free-text query. The matching
// core is a pure function (`searchSettings`) so it can be unit-tested without
// React; the hook wires in the live i18n translator and developer-mode flag.
//
// Matching rules:
//  - The query is normalised (lowercased + diacritics stripped) and split into
//    whitespace tokens. EVERY token must appear somewhere in the entry's
//    haystack (title + description + section + keywords) — i.e. AND semantics,
//    so "voice debug" narrows rather than widens.
//  - Results are ranked: a title prefix hit beats a title substring hit, which
//    beats a keyword/section hit, which beats a description-only hit. Ties fall
//    back to the registry's declaration order (stable).
// ---------------------------------------------------------------------------
import { useMemo } from 'react';

import { useDeveloperMode } from '../../../hooks/useDeveloperMode';
import { useT } from '../../../lib/i18n/I18nContext';
import { SETTINGS_SEARCH_ENTRIES, type SettingsSearchEntry } from './settingsSearchRegistry';

export interface SettingsSearchResult {
  entry: SettingsSearchEntry;
  /** Localised title for rendering. */
  title: string;
  /** Localised description for rendering (may be empty). */
  description: string;
  /** Localised section badge label. */
  section: string;
}

/** Lower-case and strip diacritics so "Tóol" matches "tool". */
export const normalize = (value: string): string =>
  value
    .normalize('NFD')
    // Strip combining diacritical marks (U+0300–U+036F) so "Tóol" matches "tool".
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .trim();

type Translate = (key: string) => string;

interface ScoredResult extends SettingsSearchResult {
  score: number;
  order: number;
}

// Higher = better match. Kept as named constants for readability.
const SCORE_TITLE_PREFIX = 4;
const SCORE_TITLE_INCLUDES = 3;
const SCORE_KEYWORD_OR_SECTION = 2;
const SCORE_DESCRIPTION = 1;
const SCORE_NONE = 0;

/**
 * Score a single entry against the already-normalised query tokens. Returns
 * `SCORE_NONE` (0) when any token is unmatched — such entries are filtered out.
 */
const scoreEntry = (
  title: string,
  description: string,
  section: string,
  keywords: string[],
  tokens: string[]
): number => {
  const nTitle = normalize(title);
  const nDescription = normalize(description);
  const nSection = normalize(section);
  const nKeywords = keywords.map(normalize);

  let best = SCORE_NONE;

  for (const token of tokens) {
    let tokenScore = SCORE_NONE;
    if (nTitle.startsWith(token)) {
      tokenScore = SCORE_TITLE_PREFIX;
    } else if (nTitle.includes(token)) {
      tokenScore = SCORE_TITLE_INCLUDES;
    } else if (nKeywords.some(k => k.includes(token)) || nSection.includes(token)) {
      tokenScore = SCORE_KEYWORD_OR_SECTION;
    } else if (nDescription.includes(token)) {
      tokenScore = SCORE_DESCRIPTION;
    }

    // AND semantics: a single unmatched token disqualifies the entry.
    if (tokenScore === SCORE_NONE) return SCORE_NONE;
    best = Math.max(best, tokenScore);
  }

  return best;
};

/**
 * Pure search over the registry. Exposed for unit testing.
 *
 * @param entries        registry entries to consider
 * @param query          raw user query
 * @param t              i18n translator
 * @param includeDevOnly whether developer-mode-only entries are eligible
 */
export const searchSettings = (
  entries: SettingsSearchEntry[],
  query: string,
  t: Translate,
  includeDevOnly: boolean
): SettingsSearchResult[] => {
  const normalizedQuery = normalize(query);
  if (!normalizedQuery) return [];

  const tokens = normalizedQuery.split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return [];

  const scored: ScoredResult[] = [];

  entries.forEach((entry, order) => {
    if (entry.devOnly && !includeDevOnly) return;

    const title = t(entry.titleKey);
    const description = entry.descriptionKey ? t(entry.descriptionKey) : '';
    const section = t(entry.sectionKey);
    const keywords = entry.keywords ?? [];

    const score = scoreEntry(title, description, section, keywords, tokens);
    if (score === SCORE_NONE) return;

    scored.push({ entry, title, description, section, score, order });
  });

  scored.sort((a, b) => (b.score !== a.score ? b.score - a.score : a.order - b.order));

  return scored.map(({ entry, title, description, section }) => ({
    entry,
    title,
    description,
    section,
  }));
};

/**
 * React hook: returns the ranked, localised, dev-gated results for `query`.
 * An empty/whitespace query yields an empty array (caller renders the normal
 * menu instead).
 */
export const useSettingsSearch = (query: string): SettingsSearchResult[] => {
  const { t } = useT();
  const developerMode = useDeveloperMode();

  return useMemo(
    () => searchSettings(SETTINGS_SEARCH_ENTRIES, query, t, developerMode),
    [query, t, developerMode]
  );
};
