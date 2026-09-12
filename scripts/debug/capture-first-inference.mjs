#!/usr/bin/env node

import fs from 'node:fs';
import http from 'node:http';
import https from 'node:https';
import path from 'node:path';

const LOOPBACK_HOSTS = new Set(['127.0.0.1', '::1', 'localhost']);

function isLoopbackHost(host) {
  return LOOPBACK_HOSTS.has(host.toLowerCase());
}

const listenHost = process.env.CAPTURE_HOST || '127.0.0.1';
const listenPort = Number.parseInt(process.env.CAPTURE_PORT || '18765', 10);
const upstream = new URL(process.env.CAPTURE_UPSTREAM || 'https://api.tinyhumans.ai');
const outputPath = path.resolve(
  process.env.CAPTURE_OUTPUT || 'target/debug-logs/first-inference-request.json'
);

// The proxy forwards the inbound `authorization` header verbatim. Binding to a
// non-loopback interface would expose that bearer to anything on the network
// that can reach this port with no auth of its own; forwarding it over a
// plaintext (`http:`) upstream that isn't itself loopback would expose it in
// transit. Both are opt-in escape hatches for a deliberate reason (a
// non-loopback CAPTURE_UPSTREAM pointed at a local mock server on `http:` is a
// normal debugging setup), gated by explicit env vars rather than silently
// allowed.
if (!isLoopbackHost(listenHost) && process.env.CAPTURE_ALLOW_REMOTE !== '1') {
  throw new Error(
    `refusing to bind CAPTURE_HOST=${listenHost}: not loopback. ` +
      'Set CAPTURE_ALLOW_REMOTE=1 to bind a non-loopback interface anyway.'
  );
}
if (
  upstream.protocol === 'http:' &&
  !isLoopbackHost(upstream.hostname) &&
  process.env.CAPTURE_ALLOW_PLAINTEXT_UPSTREAM !== '1'
) {
  throw new Error(
    `refusing to forward the authorization header to CAPTURE_UPSTREAM=${upstream.origin} over plaintext http:. ` +
      'Use an https: upstream, point at a loopback host, or set CAPTURE_ALLOW_PLAINTEXT_UPSTREAM=1 to override.'
  );
}
// `CAPTURE_ALL=1` records *every* inference request of the session, numbered, into
// `CAPTURE_ALL_DIR`. The single-shot default answers "what does the first turn
// cost"; only the sequence answers "does the cacheable prefix survive turn 2",
// which is a different question and the one a prefix cache is graded on.
const captureAll = process.env.CAPTURE_ALL === '1';
const captureAllDir = path.resolve(
  process.env.CAPTURE_ALL_DIR || 'target/debug-logs/inference-sequence'
);

if (upstream.protocol !== 'https:' && upstream.protocol !== 'http:') {
  throw new Error(`unsupported upstream protocol: ${upstream.protocol}`);
}

let captured = false;
let sequenceIndex = 0;

function isInferenceRequest(req) {
  return (
    req.method === 'POST' &&
    (req.url?.includes('/openai/v1/chat/completions') ||
      req.url?.includes('/v1/chat/completions'))
  );
}

function upstreamPath(requestUrl) {
  const basePath = upstream.pathname.replace(/\/$/, '');
  return `${basePath}${requestUrl || '/'}`;
}

const server = http.createServer((req, res) => {
  const chunks = [];
  req.on('data', chunk => chunks.push(chunk));
  req.on('end', () => {
    const body = Buffer.concat(chunks);

    if (isInferenceRequest(req)) {
      if (!captured) {
        fs.mkdirSync(path.dirname(outputPath), { recursive: true });
        fs.writeFileSync(outputPath, body);
        captured = true;
        process.stdout.write(`[capture] wrote first inference body to ${outputPath}\n`);
      }
      if (captureAll) {
        fs.mkdirSync(captureAllDir, { recursive: true });
        const name = `req-${String(sequenceIndex).padStart(3, '0')}.json`;
        fs.writeFileSync(path.join(captureAllDir, name), body);
        sequenceIndex += 1;
        process.stdout.write(`[capture] wrote ${name} (${body.length} B)\n`);
      }
    }

    const headers = { ...req.headers, host: upstream.host };
    delete headers['content-length'];
    headers['content-length'] = String(body.length);

    const transport = upstream.protocol === 'https:' ? https : http;
    const upstreamReq = transport.request(
      {
        protocol: upstream.protocol,
        hostname: upstream.hostname,
        port: upstream.port || undefined,
        method: req.method,
        path: upstreamPath(req.url),
        headers,
      },
      upstreamRes => {
        res.writeHead(upstreamRes.statusCode || 502, upstreamRes.headers);
        upstreamRes.pipe(res);
      }
    );

    upstreamReq.on('error', error => {
      process.stderr.write(`[capture] upstream error: ${error.message}\n`);
      if (!res.headersSent) res.writeHead(502, { 'content-type': 'text/plain' });
      res.end('capture proxy upstream error');
    });

    upstreamReq.end(body);
  });
});

server.listen(listenPort, listenHost, () => {
  process.stdout.write(
    `[capture] listening on http://${listenHost}:${listenPort}; forwarding to ${upstream.origin}` +
      `${captureAll ? `; recording every request under ${captureAllDir}` : ''}\n`
  );
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
