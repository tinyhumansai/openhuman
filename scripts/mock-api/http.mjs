const CORS_HEADERS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, PUT, PATCH, DELETE, OPTIONS",
  "Access-Control-Allow-Headers":
    "Content-Type, Authorization, x-device-fingerprint, x-tauri-version, x-core-version, x-ios-version, x-android-version, x-web-version",
  "Access-Control-Max-Age": "86400",
};

export function setCors(res) {
  for (const [key, value] of Object.entries(CORS_HEADERS)) {
    res.setHeader(key, value);
  }
}

export function json(res, status, body) {
  setCors(res);
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

export function html(res, status, body) {
  setCors(res);
  res.writeHead(status, { "Content-Type": "text/html; charset=utf-8" });
  res.end(body);
}

export function requestOrigin(req) {
  const host = req.headers.host || "127.0.0.1:18473";
  return `http://${host}`;
}

export function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks).toString()));
    // Don't let stream errors / aborts wedge the dispatcher waiting for
    // an end event that will never come.
    req.on("error", reject);
    req.on("aborted", () => reject(new Error("request aborted")));
  });
}

export function tryParseJson(raw) {
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

const REDACTED_HEADER_VALUE = "[REDACTED]";
const SENSITIVE_HEADER_NAMES = new Set([
  "authorization",
  "cookie",
  "set-cookie",
  "proxy-authorization",
]);

export function normalizeHeaders(headers) {
  const entries = Object.entries(headers || {});
  return Object.fromEntries(
    entries.map(([key, value]) => {
      if (SENSITIVE_HEADER_NAMES.has(String(key).toLowerCase())) {
        return [key, REDACTED_HEADER_VALUE];
      }
      return [
        key,
        Array.isArray(value) ? value.join(", ") : String(value ?? ""),
      ];
    }),
  );
}
