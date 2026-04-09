# Proxy Route Flow (`src/routes/proxy.ts`)

This document explains how proxy requests move through the backend when calling:

- `/proxy/by-id/:integrationId/{*path}`
- `/proxy/encrypted/:integrationId/{*path}`

Both endpoints let clients call third-party provider APIs (Google, Notion, etc.) through our backend while enforcing ownership checks, path restrictions, and token handling.

## High-level architecture

`src/routes/proxy.ts` is the HTTP entrypoint. It:

1. Authenticates the user via JWT
2. Applies per-user rate limiting (100 requests/minute)
3. Normalizes request params, headers, and query
4. Delegates to controller logic:
   - non-encrypted flow: `forwardByIntegrationId(...)`
   - encrypted flow: `forwardWithEncryptedTokens(...)`
5. Returns upstream status/body with a safe subset of headers

## Shared route behavior

Both routes in `src/routes/proxy.ts` share these behaviors:

- `authenticateJWT` middleware requires a valid bearer token.
- `proxyRateLimit` limits calls by authenticated user ID (falls back to IP).
- `integrationId` is validated for presence in the route handler.
- wildcard `{*path}` is normalized into a leading-slash path (for example, `v1/users/me` -> `/v1/users/me`).
- query and headers are normalized into `Record<string, string>` using `toSingleValueRecord`.

## Flow A: `/proxy/by-id/:integrationId/{*path}` (standard token storage)

### Route layer (`src/routes/proxy.ts`)

1. Extracts `integrationId` and wildcard `path`.
2. Builds a `ProxyRequest` object:
   - `method`
   - `path`
   - normalized `query`
   - normalized `headers`
   - `body`
3. Calls `forwardByIntegrationId(integrationId, userId, request)`.
4. Sets returned headers and forwards status/data to the client.

### Controller layer (`src/controllers/proxy/forward.ts`)

`forwardByIntegrationId(...)`:

1. Validates ObjectId format.
2. Loads `OAuthIntegration` by ID.
3. Verifies integration ownership (`integration.user === userId`).
4. Delegates to `forward(provider, userId, request)`.

`forward(...)`:

1. Loads provider proxy config (`getProviderProxyConfig`).
2. Enforces max body size for non-GET requests.
3. Blocks sensitive provider paths using `blockedPaths`.
4. Loads the user's provider integration.
5. Runs provider-specific request validation via forwarder hook.
6. Resolves OAuth token refresh config from provider registry.
7. Gets a valid access token through `getValidAccessToken(...)` (auto-refresh when close to expiry).
8. Resolves final upstream base URL/path.
9. Builds outgoing headers:
   - `Accept: application/json`
   - provider auth header (for example, bearer token)
   - `Content-Type: application/json` for non-GET with body
   - provider-specific header overrides via forwarder hook
10. Sends upstream request with axios (30s timeout).
11. Converts upstream `401/403` into `NotAuthorizedError`.
12. Returns upstream response with safe headers only (`content-type`, `x-request-id`, `retry-after`).

## Flow B: `/proxy/encrypted/:integrationId/{*path}` (encrypted token storage)

This is the key-split flow for integrations saved in encrypted mode.

### Route layer (`src/routes/proxy.ts`)

1. Reads `X-Encryption-Key` header (required).
2. Parses it via `parseKeyFromString(...)` into client key share bytes.
3. Validates `integrationId` and normalizes path/query/headers/body.
4. Calls `forwardWithEncryptedTokens(integrationId, userId, clientKeyShare, request)`.
5. For success: forwards status/data and safe headers.
6. For errors:
   - derives `statusCode` from error object (default `500`)
   - returns `{ success: false, error }`
   - if status is `401` or `403`, also returns:
     - `authError: true`
     - `reconnectRequired: true`

### Controller layer (`src/controllers/proxy/forwardEncrypted.ts`)

`forwardWithEncryptedTokens(...)`:

1. Validates ObjectId format.
2. Loads integration and verifies ownership.
3. Ensures integration uses `encryptionMode === 'encrypted'`.
4. Loads provider config and applies body-size and blocked-path checks.
5. Runs provider-specific validation hook.
6. Resolves provider base URL and target URL.
7. Executes request with one auth retry loop:
   - fetches decrypted valid token via `getValidAccessTokenEncrypted(...)`
   - on retry, passes `forceRefresh = true`
   - builds provider headers and sends upstream request
   - if upstream returns `401` on first attempt, retries once after forced refresh
8. Returns upstream response (status, data, safe headers).
9. Maps network/timeout failures to `BadRequestError`.

## Token lifecycle differences

- Standard route (`/by-id`):
  - uses backend token cache/token manager flow (`getValidAccessToken`)
  - refreshes according to provider refresh configuration

- Encrypted route (`/encrypted`):
  - decrypts stored token material using client key share + server share
  - refreshes and then re-encrypts/persists updated tokens through encrypted token service
  - performs one forced-refresh retry after upstream `401`

## Security and safety controls

- JWT required for all proxy calls.
- Integration ownership enforced server-side.
- Per-user rate limiting applied before forwarding.
- Provider-specific blocked paths prevent unsafe endpoint forwarding.
- Request body size limits prevent large payload abuse.
- Only safe response headers are exposed back to clients.
- Encrypted route requires client key share header.

## Typical request sequence (encrypted flow)

1. Client sends request to `/proxy/encrypted/:integrationId/...` with:
   - bearer token
   - `X-Encryption-Key`
2. Route validates inputs and delegates to encrypted controller.
3. Controller validates integration + ownership + mode.
4. Token service decrypts and refreshes token if needed.
5. Backend forwards request to provider API.
6. If provider returns `401`, backend refreshes token and retries once.
7. Backend returns provider response (or structured error with reconnect hints).

## Files involved

- `src/routes/proxy.ts`
- `src/controllers/proxy/forward.ts`
- `src/controllers/proxy/forwardEncrypted.ts`
- `src/services/oauth/providers/tokenManager.ts`
- `src/services/oauth/encryptedTokenService.ts`
- `src/services/proxy/providerConfig.ts`
- `src/services/oauth/forwarder/*`
