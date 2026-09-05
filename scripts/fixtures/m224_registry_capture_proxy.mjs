#!/usr/bin/env node
import http from 'node:http';
import { writeFileSync } from 'node:fs';

const targetBase = process.env.M224_PROXY_TARGET;
const listenPort = Number(process.env.M224_PROXY_PORT || 0);
const outputPath = process.env.M224_PROXY_LOG;

if (!targetBase || !outputPath || !listenPort) {
  console.error('m224 proxy requires M224_PROXY_TARGET, M224_PROXY_PORT, and M224_PROXY_LOG');
  process.exit(2);
}

const PAGED_QUERY_KEYS = new Set(['limit', 'cursor']);
const targetOrigin = new URL(targetBase).origin;

const ALLOWED_GET_PATTERNS = [
  { pattern: /^\/api\/v1\/kernel\/agents$/, allowPagedQuery: true },
  { pattern: /^\/api\/v1\/kernel\/agents\/[^/]+\/versions\/[1-9][0-9]*$/, allowPagedQuery: false },
  { pattern: /^\/api\/v1\/kernel\/tool-definitions$/, allowPagedQuery: true },
  {
    pattern: /^\/api\/v1\/kernel\/tool-definitions\/[^/]+\/versions\/[1-9][0-9]*$/,
    allowPagedQuery: false,
  },
  { pattern: /^\/api\/v1\/kernel\/tool-enablement$/, allowPagedQuery: false },
  {
    pattern: /^\/api\/v1\/kernel\/tool-enablement\/[^/]+\/versions\/[1-9][0-9]*$/,
    allowPagedQuery: false,
  },
  { pattern: /^\/api\/v1\/kernel\/connector-types$/, allowPagedQuery: true },
  {
    pattern: /^\/api\/v1\/kernel\/connector-types\/[^/]+\/versions\/[1-9][0-9]*$/,
    allowPagedQuery: false,
  },
  { pattern: /^\/api\/v1\/kernel\/connector-bindings$/, allowPagedQuery: true },
  {
    pattern: /^\/api\/v1\/kernel\/connector-bindings\/[^/]+\/versions\/[1-9][0-9]*$/,
    allowPagedQuery: false,
  },
];

const entries = [];

function persist() {
  writeFileSync(outputPath, `${JSON.stringify(entries, null, 2)}\n`, 'utf8');
}

function sanitizePath(rawUrl) {
  const parsed = new URL(rawUrl, targetBase);
  const safe = new URL(parsed.pathname, targetBase);
  const cursorPresent = parsed.searchParams.has('cursor');
  const limit = parsed.searchParams.get('limit');
  if (limit) {
    safe.searchParams.set('limit', limit);
  }
  if (cursorPresent) {
    safe.searchParams.delete('cursor');
  }
  return {
    path: `${safe.pathname}${safe.search}`,
    cursorPresent,
  };
}

function parsePinnedRequestUrl(rawUrl) {
  if (!rawUrl.startsWith('/') || rawUrl.startsWith('//')) {
    return null;
  }
  const requestUrl = new URL(rawUrl, targetBase);
  return requestUrl.origin === targetOrigin ? requestUrl : null;
}

function validatePagedQuery(search) {
  if (!search) {
    return true;
  }
  if (!search.startsWith('?')) {
    return false;
  }
  const rawPairs = search.slice(1).split('&');
  if (rawPairs.some(segment => segment.length === 0)) {
    return false;
  }
  const searchParams = new URLSearchParams(search);
  const seenKeys = new Set();
  for (const [key] of searchParams.entries()) {
    if (!key || !PAGED_QUERY_KEYS.has(key) || seenKeys.has(key)) {
      return false;
    }
    seenKeys.add(key);
  }
  return seenKeys.size === rawPairs.length;
}

function assertAllowed(method, requestUrl) {
  if (method !== 'GET') {
    return false;
  }
  const matched = ALLOWED_GET_PATTERNS.find(({ pattern }) => pattern.test(requestUrl.pathname));
  if (!matched) {
    return false;
  }
  return matched.allowPagedQuery
    ? validatePagedQuery(requestUrl.search)
    : requestUrl.search.length === 0;
}

const server = http.createServer(async (req, res) => {
  const rawUrl = req.url ?? '/';
  const requestUrl = parsePinnedRequestUrl(rawUrl);
  const { path: safePath, cursorPresent } = sanitizePath(rawUrl);
  if (!requestUrl || !assertAllowed(req.method ?? '', requestUrl)) {
    entries.push({
      method: req.method ?? 'UNKNOWN',
      path: safePath,
      cursorPresent,
      statusCode: 405,
      blocked: true,
    });
    persist();
    res.writeHead(405, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ detail: { code: 'proxy_disallowed', message: 'request not allowed' } }));
    return;
  }

  const forwardHeaders = { ...req.headers };
  // Forward auth unchanged to the disposable Core, but never retain
  // authorization values in the artifact log.
  delete forwardHeaders.host;
  delete forwardHeaders['content-length'];

  const upstream = await fetch(requestUrl, {
    method: req.method,
    headers: forwardHeaders,
  });
  entries.push({
    method: req.method ?? 'GET',
    path: safePath,
    cursorPresent,
    statusCode: upstream.status,
  });
  persist();

  res.writeHead(upstream.status, {
    'content-type': upstream.headers.get('content-type') ?? 'application/json',
  });
  res.end(Buffer.from(await upstream.arrayBuffer()));
});

server.listen(listenPort, '127.0.0.1', () => {
  persist();
  console.log(`m224-registry-proxy listening on 127.0.0.1:${listenPort}`);
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    persist();
    server.close(() => process.exit(0));
  });
}
