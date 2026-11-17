import { json } from "../http.mjs";

// Gap fill: version-check ping used by daemonHealthService.
export function handleVersion(ctx) {
  const { method, url, res } = ctx;

  // The backend liveness probe the app runs before an OAuth sign-in
  // (`services/backendHealth.ts`); the real backend answers `{"status":"ok"}`.
  if (method === "GET" && /^\/health\/?(\?.*)?$/.test(url)) {
    json(res, 200, { status: "ok" });
    return true;
  }

  if (/^\/version-check\/?(\?.*)?$/.test(url)) {
    if (method === "GET" || method === "POST") {
      json(res, 200, {
        success: true,
        data: {
          ok: true,
          serverVersion: "0.0.0-mock",
          minClientVersion: "0.0.0",
        },
      });
      return true;
    }
  }

  return false;
}
