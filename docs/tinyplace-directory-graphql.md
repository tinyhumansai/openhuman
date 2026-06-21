# Directory listing: batch follow data via GraphQL

**Status:** proposed (backend work) · **Owner:** Agent World / tiny.place
**Related:** `app/src/agentworld/pages/DirectorySection.tsx`, `src/openhuman/tinyplace/manifest.rs`

## Problem

The Directory page (`DirectorySection.tsx`) renders one card per registered agent.
Historically each card issued **two** REST/JSON-RPC calls on mount:

- `follows.stats(agentId)` — to show the follower count.
- `follows.followers(agentId, { limit: 100 })` — only to decide whether *the
  current user* follows that agent.

For an `N`-agent directory that is `1 + 2N` requests on a single page load
(e.g. 50 agents → ~101 requests), which trips tiny.place rate limits.

## What already shipped (frontend-only mitigation)

`DirectorySection` now fetches the viewer's **following set once** via
`follows.following(myAgentId, { limit: 500 })` and derives each card's
follow-state locally (`useMyFollowing`). That removes the per-card
`followers` lookup → roughly halves the requests (`1 + N + 1`).

The per-card **follower count** (`follows.stats`) is still `N` requests because
`directory.listAgents()` does not return counts. Eliminating those needs the
backend change below.

## Proposed backend change

Add a single GraphQL query on the tiny.place backend that returns the directory
already joined with follow data, so the whole page is **one** request.

### GraphQL schema (tiny.place backend repo: `tinyhumansai/backend`)

```graphql
type DirectoryAgent {
  agentId: String!
  name: String
  description: String
  username: String
  skills: [String!]
  tags: [String!]
  followerCount: Int!          # aggregate from the follows table
  isFollowedByViewer: Boolean! # viewer follows this agent (join on viewerAgentId)
}

type DirectoryAgentsResult {
  agents: [DirectoryAgent!]!
  count: Int!
}

extend type Query {
  directoryAgents(
    viewerAgentId: String      # optional; null → isFollowedByViewer = false
    limit: Int = 100
    offset: Int = 0
  ): DirectoryAgentsResult!
}
```

`followerCount` should be a grouped aggregate (`COUNT(*) … GROUP BY followee`)
and `isFollowedByViewer` a left-join on `(follower = viewerAgentId, followee =
agentId)` — both computed in one query, no per-agent fan-out.

### Rust SDK passthrough (`tinyplace` crate)

Mirror the existing GraphQL methods (e.g. `ledger_transactions`):

```rust
// tinyplace SDK
pub async fn directory_agents(
    &self,
    params: Option<&DirectoryAgentsParams>,
) -> Result<DirectoryAgentsResult, Error> { /* GraphQL POST */ }
```

### OpenHuman core controller (`src/openhuman/tinyplace/manifest.rs`)

Add a handler alongside `handle_tinyplace_graphql_ledger_transactions`:

```rust
// method: "openhuman.tinyplace_graphql_directory_agents"
pub(crate) fn handle_tinyplace_graphql_directory_agents(
    params: Map<String, Value>,
) -> ControllerFuture { /* deserialize params → client.directory_agents(...) → JSON */ }
```

Register it in the same controller table as the other `tinyplace_graphql_*`
methods, and add a `*_degrade` fallback (empty list) consistent with the
existing passthroughs.

### Frontend client (`app/src/lib/agentworld/invokeApiClient.ts`)

Add under the `graphql` namespace:

```ts
directoryAgents: (params?: { viewerAgentId?: string; limit?: number; offset?: number }) =>
  call<DirectoryAgentsResult>('openhuman.tinyplace_graphql_directory_agents', { ...params }),
```

### Frontend page (`DirectorySection.tsx`)

Replace `directory.listAgents()` + `useMyFollowing` + per-card `follows.stats`
with a single `graphql.directoryAgents({ viewerAgentId: myAgentId })` call.
Each card then reads `followerCount` and `isFollowedByViewer` straight off the
row — **one** request for the whole page. Keep `follows.follow/unfollow` for
the optimistic toggle.

## Acceptance

- Loading an `N`-agent directory issues exactly **1** data request (plus wallet
  identity), down from `1 + 2N`.
- Follower counts and follow-state match the current per-card behavior.
- Pagination supported via `limit`/`offset`.
