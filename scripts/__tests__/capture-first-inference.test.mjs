import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path, { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { after, before, test } from "node:test";

const HERE = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(HERE, "..", "debug", "capture-first-inference.mjs");

test("capture proxy --help prints usage without opening a listener", () => {
  const helpEnv = { ...process.env };
  delete helpEnv.FORCE_COLOR;
  const result = spawnSync(process.execPath, [SCRIPT, "--help"], {
    encoding: "utf8",
    env: helpEnv,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(
    result.stdout,
    /Usage: node scripts\/debug\/capture-first-inference\.mjs/,
  );
  assert.match(result.stdout, /CAPTURE_UPSTREAM/);
  assert.match(result.stdout, /pnpm debug capture/);
  assert.equal(result.stderr, "");
});

test("capture proxy refuses a non-loopback bind and a plaintext remote upstream by default", () => {
  const bind = spawnSync(process.execPath, [SCRIPT], {
    encoding: "utf8",
    env: { ...process.env, CAPTURE_HOST: "0.0.0.0", CAPTURE_PORT: "0" },
  });
  assert.notEqual(bind.status, 0);
  assert.match(bind.stderr, /refusing to bind CAPTURE_HOST=0\.0\.0\.0/);

  const upstream = spawnSync(process.execPath, [SCRIPT], {
    encoding: "utf8",
    env: {
      ...process.env,
      CAPTURE_UPSTREAM: "http://example.com",
      CAPTURE_PORT: "0",
    },
  });
  assert.notEqual(upstream.status, 0);
  assert.match(upstream.stderr, /plaintext http:/);
});

// A stand-in for the backend / OpenRouter: streams an OpenAI-compatible SSE
// response carrying the `provider` field and a final usage block, the two
// things the summary line exists to surface. Non-inference routes answer
// plain JSON so the passthrough can be checked too.
function startMockUpstream() {
  const requests = [];
  const server = http.createServer((req, res) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const body = Buffer.concat(chunks).toString("utf8");
      requests.push({
        method: req.method,
        url: req.url,
        headers: req.headers,
        body,
      });
      if (req.url === "/openai/v1/chat/completions") {
        if (body.includes('"model":"boom"')) {
          res.writeHead(503, { "content-type": "text/html" });
          res.end(
            "<html><head><title>503 Service Temporarily Unavailable</title></head></html>",
          );
          return;
        }
        res.writeHead(200, { "content-type": "text/event-stream" });
        const chunk = (delta) =>
          `data: ${JSON.stringify({
            id: "gen-1",
            provider: "StreamLake",
            choices: [{ index: 0, delta }],
          })}\n\n`;
        res.write(chunk({ role: "assistant", content: "Hel" }));
        setTimeout(() => {
          res.write(chunk({ content: "lo" }));
          res.write(
            `data: ${JSON.stringify({
              id: "gen-1",
              provider: "StreamLake",
              choices: [],
              usage: {
                prompt_tokens: 12344,
                completion_tokens: 2,
                prompt_tokens_details: { cached_tokens: 12288 },
              },
            })}\n\n`,
          );
          res.write("data: [DONE]\n\n");
          res.end();
        }, 30);
        return;
      }
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ ok: true, path: req.url }));
    });
  });
  return new Promise((resolveStart) => {
    server.listen(0, "127.0.0.1", () =>
      resolveStart({ server, port: server.address().port, requests }),
    );
  });
}

function startProxy(env) {
  const child = spawn(process.execPath, [SCRIPT], {
    env: { ...process.env, ...env },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (d) => (stdout += d));
  child.stderr.on("data", (d) => (stderr += d));
  const ready = new Promise((resolveReady, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`proxy did not start:\n${stdout}\n${stderr}`)),
      10_000,
    );
    const check = () => {
      const m = stdout.match(/listening on http:\/\/127\.0\.0\.1:(\d+)/);
      if (m) {
        clearTimeout(timer);
        resolveReady(Number(m[1]));
      } else if (child.exitCode !== null) {
        clearTimeout(timer);
        reject(new Error(`proxy exited ${child.exitCode}:\n${stderr}`));
      } else {
        setTimeout(check, 25);
      }
    };
    check();
  });
  return { child, ready, output: () => stdout, errors: () => stderr };
}

async function post(port, urlPath, body, headers = {}) {
  const res = await fetch(`http://127.0.0.1:${port}${urlPath}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: "Bearer secret-token",
      ...headers,
    },
    body: JSON.stringify(body),
  });
  return { status: res.status, text: await res.text() };
}

// The proxy writes its stdout summary line and closes the HTTP response from
// the same synchronous handler, in that order, but the two travel to this
// test over different channels — a pipe for stdout, a loopback socket for the
// response — with no ordering guarantee between them once they leave the
// child process. `fetch()` resolving is therefore not proof the stdout bytes
// have arrived yet; poll briefly instead of asserting the instant it returns.
async function waitForOutput(getOutput, pattern, timeoutMs = 2000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const output = getOutput();
    if (pattern.test(output)) return output;
    if (Date.now() >= deadline) return output;
    await new Promise((r) => setTimeout(r, 10));
  }
}

let upstream;
let proxy;
let workDir;

before(async () => {
  upstream = await startMockUpstream();
  workDir = fs.mkdtempSync(path.join(os.tmpdir(), "capture-proxy-"));
  proxy = startProxy({
    CAPTURE_PORT: "0",
    CAPTURE_UPSTREAM: `http://127.0.0.1:${upstream.port}`,
    CAPTURE_ALL: "1",
    CAPTURE_RESPONSES: "1",
    CAPTURE_ALL_DIR: path.join(workDir, "seq"),
    CAPTURE_OUTPUT: path.join(workDir, "first.json"),
    CAPTURE_LOG: path.join(workDir, "capture.jsonl"),
  });
});

after(() => {
  proxy?.child.kill("SIGTERM");
  upstream?.server.close();
  if (workDir) fs.rmSync(workDir, { recursive: true, force: true });
});

test("capture proxy forwards an inference call, dumps the body, and summarises the response", async () => {
  const port = await proxy.ready;
  const request = {
    model: "z-ai/glm-5.3-flash",
    stream: true,
    prompt_cache_key: "tap-25675927a3f2160d",
    thread_id: "thread-1",
    tools: [{ type: "function", function: { name: "web_search_tool" } }],
    messages: [
      { role: "system", content: "sys" },
      { role: "user", content: "hi" },
    ],
  };
  const reply = await post(port, "/openai/v1/chat/completions", request);
  assert.equal(reply.status, 200);
  assert.match(
    reply.text,
    /"content":"Hel"/,
    "the stream reaches the client untouched",
  );
  assert.match(reply.text, /\[DONE\]/);
  assert.equal(
    fs.readFileSync(path.join(workDir, "seq", "res-000.txt"), "utf8"),
    reply.text,
    "the saved successful response must match the stream forwarded to the core",
  );
  for (const file of [
    "first.json",
    "seq/req-000.json",
    "seq/res-000.txt",
    "capture.jsonl",
  ]) {
    assert.equal(
      fs.statSync(path.join(workDir, file)).mode & 0o777,
      0o600,
      file,
    );
  }

  // Forwarded verbatim, bearer included, to the upstream path.
  const seen = upstream.requests.find(
    (r) => r.url === "/openai/v1/chat/completions",
  );
  assert.ok(seen, "upstream received the inference call");
  assert.equal(seen.headers.authorization, "Bearer secret-token");
  assert.deepEqual(JSON.parse(seen.body), request);

  // Request dumps: first-body file and the numbered sequence.
  assert.deepEqual(
    JSON.parse(fs.readFileSync(path.join(workDir, "first.json"), "utf8")),
    request,
  );
  assert.deepEqual(
    JSON.parse(
      fs.readFileSync(path.join(workDir, "seq", "req-000.json"), "utf8"),
    ),
    request,
  );

  // Summary: one JSONL record and one stdout line with the routing facts.
  const record = fs
    .readFileSync(path.join(workDir, "capture.jsonl"), "utf8")
    .trim()
    .split("\n")
    .map((l) => JSON.parse(l))
    .find((r) => r.seq === 0);
  assert.ok(record, "summary record written");
  assert.equal(record.status, 200);
  assert.equal(record.model, "z-ai/glm-5.3-flash");
  assert.equal(record.messages, 2);
  assert.equal(record.tools, 1);
  assert.equal(record.prompt_cache_key, "tap-25675927a3f2160d");
  assert.equal(record.thread_id, "thread-1");
  assert.equal(record.provider, "StreamLake");
  assert.equal(record.prompt_tokens, 12344);
  assert.equal(record.cached_tokens, 12288);
  assert.equal(record.error, null);
  assert.ok(
    record.ttfb_ms >= 0 && record.total_ms >= record.ttfb_ms,
    JSON.stringify(record),
  );
  assert.equal(record.stream, true);
  // The stub streams visible content straight away, so the first token is the
  // first content, and both land no earlier than the first byte.
  assert.ok(record.first_token_ms >= record.ttfb_ms, JSON.stringify(record));
  assert.equal(record.first_content_ms, record.first_token_ms);
  const summaryLine =
    /\[capture\] #000 200 model=z-ai\/glm-5\.3-flash msgs=2 tools=1 served_by=StreamLake ttfb=\d+\.\d\ds total=\d+\.\d\ds prompt=12344 cached=12288 cache_key=tap-25675927a3f2160d stream=yes ttft=\d+\.\d\ds ttfc=\d+\.\d\ds thread=thread-1/;
  assert.match(await waitForOutput(proxy.output, summaryLine), summaryLine);
});

test("capture proxy forwards Socket.IO WebSocket upgrades and both socket directions", async () => {
  const port = await proxy.ready;
  let seen;
  let upstreamSocket;
  const onUpgrade = (req, socket, head) => {
    upstreamSocket = socket;
    seen = {
      url: req.url,
      authorization: req.headers.authorization,
      host: req.headers.host,
    };
    socket.write(
      "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
    );
    if (head.length) socket.write(head);
    socket.on("data", (chunk) => socket.write(chunk));
  };
  upstream.server.on("upgrade", onUpgrade);

  let clientSocket;
  try {
    await new Promise((resolve, reject) => {
      const request = http.request({
        hostname: "127.0.0.1",
        port,
        path: "/socket.io/?EIO=4&transport=websocket",
        headers: {
          connection: "Upgrade",
          upgrade: "websocket",
          authorization: "Bearer websocket-test",
        },
      });
      const timer = setTimeout(
        () => request.destroy(new Error("WebSocket upgrade did not complete")),
        1500,
      );
      request.on("upgrade", (response, socket) => {
        clientSocket = socket;
        if (response.statusCode !== 101) {
          clearTimeout(timer);
          reject(new Error(`unexpected upgrade status ${response.statusCode}`));
          return;
        }
        socket.on("data", (chunk) => {
          if (chunk.toString() === "socket-ping") {
            clearTimeout(timer);
            resolve();
          }
        });
        socket.write("socket-ping");
      });
      request.on("response", (response) => {
        clearTimeout(timer);
        reject(new Error(`upgrade returned HTTP ${response.statusCode}`));
      });
      request.on("error", (error) => {
        clearTimeout(timer);
        reject(error);
      });
      request.end();
    });
    assert.deepEqual(seen, {
      url: "/socket.io/?EIO=4&transport=websocket",
      authorization: "Bearer websocket-test",
      host: `127.0.0.1:${upstream.port}`,
    });
  } finally {
    clientSocket?.destroy();
    upstreamSocket?.destroy();
    upstream.server.off("upgrade", onUpgrade);
  }
});

test("capture proxy records a non-2xx inference response body and names the error", async () => {
  const port = await proxy.ready;
  const reply = await post(port, "/openai/v1/chat/completions", {
    model: "boom",
    messages: [],
  });
  assert.equal(reply.status, 503, "status passes through");

  const records = fs
    .readFileSync(path.join(workDir, "capture.jsonl"), "utf8")
    .trim()
    .split("\n")
    .map((l) => JSON.parse(l));
  const record = records.find((r) => r.model === "boom");
  assert.ok(record);
  assert.equal(record.status, 503);
  assert.match(record.error, /503 Service Temporarily Unavailable/);
  assert.ok(
    fs.existsSync(record.response_body),
    "error body saved next to the request dumps",
  );
  assert.match(fs.readFileSync(record.response_body, "utf8"), /<html>/);
});

test("capture proxy passes non-inference routes through without summarising them", async () => {
  const port = await proxy.ready;
  const reply = await post(port, "/auth/me", { probe: true });
  assert.equal(reply.status, 200);
  assert.deepEqual(JSON.parse(reply.text), { ok: true, path: "/auth/me" });
  const lines = fs
    .readFileSync(path.join(workDir, "capture.jsonl"), "utf8")
    .trim()
    .split("\n");
  assert.ok(
    lines.every((l) => !l.includes("/auth/me")),
    "no summary record for a non-inference route",
  );
  assert.doesNotMatch(proxy.output(), /auth\/me/);
});
