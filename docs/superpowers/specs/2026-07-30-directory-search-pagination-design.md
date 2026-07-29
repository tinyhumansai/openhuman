# Directory search + pagination (Tiny Place §6)

Sub-slice of the Tiny Place audit epic **#4776 §6 (Directory)**. The audit found
`DirectorySection` "largely non-interactive": card-click→profile-modal and
follow already landed (#4927), but **search, type filter, and pagination**
remain unwired even though the backend + vendored SDK fully support them. This
spec covers **search + pagination** (type filter is deliberately out of scope).

## Problem

`DirectorySection.tsx` fetches the whole directory once via
`apiClient.graphql.agents()` with **no arguments**:

- No way to search by handle/name, though `graphql.agents({ q })` maps `q` →
  the GraphQL `query` variable server-side (`vendor/tinyplace/.../graphql.rs`).
- No pagination: the SDK query threads `limit`/`offset`, but the UI sends
  neither, so large directories render a single unbounded (backend-capped) page
  with no "Load more".

Both gaps are **frontend-only** — the core handler
(`handle_tinyplace_graphql_agents`) already deserializes and forwards
`AgentQueryParams`.

## Solution

Rework `DirectorySection` to own a search query + offset-paginated list,
mirroring the existing **Feed/Ledger "Load more"** pattern already in this
directory (`FeedSection.tsx` `loadMore`, `LedgerSection.tsx` `loadMore`).

### Search (server-side, debounced)

- A labeled search `<input>` at the top of the section.
- Debounce ~300ms; the debounced query drives
  `graphql.agents({ q, limit: PAGE_SIZE, offset: 0 })`.
- Empty query = browse-all (today's behavior).
- A query change **resets** pagination to `offset: 0` and replaces the list
  (does not append).

### Pagination (offset "Load more")

- `DIRECTORY_PAGE_SIZE = 24` (divides 1/2/3-column grids evenly).
- Initial/query-change fetch: `offset: 0`. "Load more" fetches
  `offset: nextOffset` and **appends**.
- Dedupe appended agents by `agentId` (a mutation-shifted offset must not
  produce duplicate React keys / double rows — the exact guard Feed/Ledger use).
- End-of-list = "page shorter than `PAGE_SIZE`" (robust; no reliance on the
  optional `count` field).

### State model

Extend the `useDirectoryAgents` hook to accept the debounced query and own:
accumulated `agents`, `nextOffset`, `hasMore`, `loadingMore`, `loadMoreError`.
Keep the existing `loading` / `payment_required` / `error` / `ok` states.
Refetch-from-zero when the query changes; append on load-more. Reentry into
`loadMore` is guarded by disabling the button while `loadingMore`.

### Types / i18n / a11y

- Add `offset?: number` to the frontend `AgentQueryParams` interface
  (`invokeApiClient.ts`) — it currently declares `cursor` only; the SDK uses
  `offset`.
- New UI text goes through `useT()` (the section is currently hardcoded; new
  strings comply with the i18n rule, existing ones are left untouched to avoid
  scope creep). New keys added to `en.ts` **and all 14 locale files**, no em
  dashes:
  `agentWorld.directory.searchLabel`, `.searchPlaceholder`, `.noResults`,
  `.loadMore`, `.loadingMore`, `.loadMoreError`.
- Search input is labeled; the existing card keyboard-nav guard (#4927) is
  preserved.

## Out of scope

- Type filter (agent / human / org).
- "Send DM" from a directory card.
- Converting the section's pre-existing hardcoded strings to i18n.

## Tests (Vitest, TDD)

Extend `DirectorySection.test.tsx`:

1. Typing a query calls `graphql.agents` with `{ q }` (after debounce) and
   replaces the list; offset resets to 0.
2. "Load more" fetches the next offset and **appends**, deduping by `agentId`.
3. A page shorter than `PAGE_SIZE` hides "Load more".
4. Empty-results state renders for a no-match query.
5. A failed "Load more" surfaces `loadMoreError` and keeps existing rows.

## Debug logging

Namespaced `debug('agentworld:directory')` already present — add entry/exit +
field logs for: first-page fetch (`q`, limit), load-more (`offset`, received,
fresh, hasMore), and load-more failure.
