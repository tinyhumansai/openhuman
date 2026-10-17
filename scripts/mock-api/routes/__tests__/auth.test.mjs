import assert from "node:assert/strict";
import test from "node:test";

import {
  clearRequestLog,
  getRequestLog,
  resetMockBehavior,
  startMockServer,
  stopMockServer,
} from "../../index.mjs";
import { loopbackSignInRedirect } from "../auth.mjs";
import { MOCK_JWT } from "../../state.mjs";

test.beforeEach(async () => {
  await stopMockServer();
  resetMockBehavior();
});

test.afterEach(async () => {
  await stopMockServer();
});

test("signs a loopback redirectUri back in with a session JWT and the echoed state", () => {
  const location = loopbackSignInRedirect(
    "/auth/github/login?redirectUri=http%3A%2F%2Flocalhost%3A1420%2F__dev-auth&state=nonce-1",
  );
  const target = new URL(location);
  assert.equal(
    target.origin + target.pathname,
    "http://localhost:1420/__dev-auth",
  );
  assert.equal(target.searchParams.get("token"), MOCK_JWT);
  assert.equal(target.searchParams.get("key"), "auth");
  assert.equal(target.searchParams.get("state"), "nonce-1");
});

test("ignores a missing, non-loopback or non-http redirectUri", () => {
  assert.equal(loopbackSignInRedirect("/auth/github/login"), null);
  assert.equal(
    loopbackSignInRedirect(
      "/auth/github/login?redirectUri=http%3A%2F%2Fevil.example%2Fcb",
    ),
    null,
  );
  assert.equal(
    loopbackSignInRedirect(
      "/auth/github/login?redirectUri=openhuman%3A%2F%2Fauth",
    ),
    null,
  );
});

test("serves the provider-login redirect and the /health probe", async () => {
  clearRequestLog();
  const started = await startMockServer(18576, { retryIfInUse: true });
  const baseUrl = `http://127.0.0.1:${started.port}`;

  const login = await fetch(
    `${baseUrl}/auth/github/login?redirectUri=${encodeURIComponent("http://127.0.0.1:5173/__dev-auth")}`,
    { redirect: "manual" },
  );
  assert.equal(login.status, 302);
  assert.match(
    login.headers.get("location"),
    /^http:\/\/127\.0\.0\.1:5173\/__dev-auth\?token=/,
  );

  // Without a redirectUri the legacy mock-oauth page is unchanged.
  const legacy = await fetch(`${baseUrl}/auth/github/login`, {
    redirect: "manual",
  });
  assert.equal(legacy.status, 302);
  assert.equal(legacy.headers.get("location"), `${baseUrl}/mock-oauth`);

  const health = await fetch(`${baseUrl}/health`);
  assert.equal(health.status, 200);
  assert.deepEqual(await health.json(), { status: "ok" });
  assert.deepEqual(
    getRequestLog().map(({ method, url, status }) => ({
      method,
      path: url,
      status,
    })),
    [
      {
        method: "GET",
        path: "/auth/github/login?redirectUri=http%3A%2F%2F127.0.0.1%3A5173%2F__dev-auth",
        status: 302,
      },
      { method: "GET", path: "/auth/github/login", status: 302 },
      { method: "GET", path: "/health", status: 200 },
    ],
  );
});
