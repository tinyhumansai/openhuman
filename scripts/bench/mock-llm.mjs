#!/usr/bin/env node
/**
 * Mock LLM + backend endpoint for the agent-scale benchmark tier.
 *
 * This stands in for `api.tinyhumans.ai` so a NORMALLY-BUILT `openhuman-core`
 * can run real agent turns with no network and no special cargo features. The
 * core reaches it because `config.api_url` (or `BACKEND_URL`) feeds both
 * `effective_api_url` (inference, embeddings) and `effective_backend_api_url`
 * (telemetry), so one base URL captures every backend call a turn makes:
 *
 *   POST /openai/v1/chat/completions   the turn itself (non-streaming)
 *   POST /openai/v1/embeddings         memory recall/write, when it fires
 *   POST /telemetry/langfuse/ingestion after every turn, unless disabled
 *
 * Two properties matter for a benchmark and are worth stating, because both
 * are easy to lose in a later edit:
 *
 * 1. REPLIES ARE DERIVED FROM THE REQUEST, NOT FROM STORED STATE. How deep a
 *    turn is comes from counting `role: "tool"` messages in the body that
 *    arrived. There is no session map, so N concurrent turns cannot interleave
 *    into each other's scripts and the mock holds nothing that grows with load.
 *    The only mutable state is a handful of integer counters.
 *
 * 2. THAT IS ALSO WHAT KEEPS THE MOCK OUT OF THE MEASUREMENT. We are trying to
 *    attribute RSS growth to the core. A mock that retained a request log would
 *    grow too, and (being a separate process) would not show up in the core's
 *    numbers — but it would eventually change the machine's memory pressure and
 *    contaminate the run. Do not add request retention here; if you need to
 *    inspect traffic, add a counter or log to stderr.
 *
 * Usage:
 *   node scripts/bench/mock-llm.mjs --port 18700 [options]
 *
 * Options:
 *   --port <n>          listen port. MUST NOT be one of 11434/8000/8080/1234/8888:
 *                       the core classifies those as local-AI endpoints and
 *                       routes around them (see LOCAL_AI_PORTS in src/api/config.rs).
 *   --latency-ms <n>    mean added latency per completion (default 0)
 *   --jitter-ms <n>     deterministic +/- jitter around that mean (default 0)
 *   --tool-depth <n>    tool calls to emit before the final answer (default 0)
 *   --reply-chars <n>   assistant reply size, to vary serde/alloc pressure (default 240)
 *   --fail-rate <n>     fraction [0,1) of completions answered 500, to exercise
 *                       the core's retry/error path (default 0)
 *   --embed-dims <n>    embedding width (default 1024). MUST match what the
 *                       memory store expects — a mismatch is only a warning,
 *                       and the run silently stores chunks without vectors.
 */

import http from 'node:http';

const LOCAL_AI_PORTS = new Set([11434, 8000, 8080, 1234, 8888]);

// Tools we are willing to drive, most preferred first. Both are read-only and
// cheap, so a deep tool loop stresses the harness rather than the filesystem.
// The mock only ever names a tool the core actually offered in the request, so
// an unknown name here is inert rather than an error.
const TOOL_PREFERENCE = ['memory_search', 'glob'];

/**
 * Background routes the core polls that are not part of a turn.
 *
 * Left un-stubbed these 404, and the core handles that gracefully — but not
 * freely: `/teams/me/usage` routes its 404 through the observability
 * error-reporting path, and the rest log warnings. That is CPU and allocation
 * spent on failure handling rather than on agent work, in a run whose entire
 * purpose is to attribute CPU and allocation. Answering them with empty,
 * well-shaped payloads keeps the process on its normal path.
 *
 * These are counted separately from `unknownRoutes` so the distinction between
 * "deliberately stubbed" and "we did not anticipate this" stays visible.
 */
const ANCILLARY_ROUTES = {
  'GET /teams/me/usage': () => ({
    success: true,
    data: { credits: 1_000_000, used: 0, plan: 'bench' },
  }),
  'GET /agent-integrations/composio/connections': () => ({ success: true, data: [] }),
  'GET /agent-integrations/composio/toolkits': () => ({ success: true, data: [] }),
  'GET /orchestration/v1/sessions': () => ({ success: true, data: [] }),
};

function parseArgs(argv) {
  const opts = {
    port: 18700,
    latencyMs: 0,
    jitterMs: 0,
    toolDepth: 0,
    replyChars: 240,
    failRate: 0,
    embedDims: 1024,
  };
  const numeric = {
    '--port': 'port',
    '--latency-ms': 'latencyMs',
    '--jitter-ms': 'jitterMs',
    '--tool-depth': 'toolDepth',
    '--reply-chars': 'replyChars',
    '--fail-rate': 'failRate',
    '--embed-dims': 'embedDims',
  };
  for (let i = 2; i < argv.length; i += 1) {
    const key = numeric[argv[i]];
    if (!key) throw new Error(`unknown argument: ${argv[i]}`);
    const raw = argv[i + 1];
    i += 1;
    const value = Number(raw);
    if (!Number.isFinite(value)) {
      throw new Error(`${argv[i - 1]} expects a number, got: ${raw}`);
    }
    opts[key] = value;
  }
  if (LOCAL_AI_PORTS.has(opts.port)) {
    throw new Error(
      `--port ${opts.port} is in the core's LOCAL_AI_PORTS set; the core would ` +
        `treat this as a local-AI endpoint and not route managed inference here. ` +
        `Pick another port.`,
    );
  }
  if (opts.failRate < 0 || opts.failRate >= 1) {
    throw new Error(`--fail-rate must be in [0, 1), got ${opts.failRate}`);
  }
  return opts;
}

/**
 * Deterministic hash of a string, so latency and failure injection are
 * reproducible across runs without a shared RNG (which would be both a lock
 * and a source of cross-request coupling).
 */
function hash32(str) {
  let h = 0x811c9dc5;
  for (let i = 0; i < str.length; i += 1) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

/** Stable [0,1) from a seed. */
function unitFrom(seed) {
  return (hash32(String(seed)) % 1_000_000) / 1_000_000;
}

// Retry counters are keyed by request content. Distinct concurrent turns cannot
// perturb one another, while a retry of the same request still gets a new draw.
const attemptsByRequest = new Map();
const MAX_TRACKED_REQUESTS = 4096;

/** Return the next attempt for a request while bounding benchmark-side state. */
function nextRequestAttempt(requestKey) {
  const attempt = (attemptsByRequest.get(requestKey) ?? 0) + 1;
  // Refresh insertion order so active retrying requests are evicted last.
  attemptsByRequest.delete(requestKey);
  attemptsByRequest.set(requestKey, attempt);
  if (attemptsByRequest.size > MAX_TRACKED_REQUESTS) {
    // Eviction intentionally permits a very old request to restart at attempt
    // 1. Core retries are immediate, so their refreshed keys remain resident;
    // preserving every completed request across an unbounded duration run
    // would make the mock itself accumulate benchmark-distorting state.
    attemptsByRequest.delete(attemptsByRequest.keys().next().value);
  }
  return attempt;
}

const stats = {
  startedAt: Date.now(),
  completions: 0,
  toolCallsEmitted: 0,
  finalAnswers: 0,
  embeddings: 0,
  telemetry: 0,
  ancillary: 0,
  injectedFailures: 0,
  unknownRoutes: 0,
  malformedRequests: 0,
};

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on('data', (chunk) => {
      size += chunk.length;
      // A runaway body would make the mock the memory problem. Cap it.
      if (size > 64 * 1024 * 1024) {
        reject(new Error('request body exceeded 64 MiB'));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });
}

function sendJson(res, status, payload) {
  const body = JSON.stringify(payload);
  res.writeHead(status, {
    'content-type': 'application/json',
    'content-length': Buffer.byteLength(body),
  });
  res.end(body);
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Pick a tool to call from the ones the core offered. Returns null when the
 * request carried no tools, which is the correct cue to answer in plain text
 * rather than inventing a call the agent cannot dispatch.
 */
function pickTool(tools) {
  if (!Array.isArray(tools) || tools.length === 0) return null;
  const names = new Set(
    tools.map((t) => t?.function?.name).filter((n) => typeof n === 'string'),
  );
  for (const preferred of TOOL_PREFERENCE) {
    if (names.has(preferred)) return preferred;
  }
  return null;
}

/** Arguments that are valid for the tools we are willing to drive. */
function argumentsFor(toolName) {
  switch (toolName) {
    case 'memory_search':
      return { query: 'benchmark probe' };
    case 'glob':
      return { pattern: '*.md' };
    default:
      return {};
  }
}

// A small vocabulary the reply builder samples from. Replies get stored as
// memories, so if every reply were the same string the whole corpus would
// collapse to one distinct document — fine for throughput, useless for asking
// which memories a retrieval change surfaces.
const VOCAB = [
  'invoice', 'deployment', 'kubernetes', 'roadmap', 'latency', 'onboarding',
  'refund', 'schema', 'migration', 'webhook', 'billing', 'timezone',
  'passport', 'itinerary', 'recipe', 'mortgage', 'vaccination', 'landlord',
  'guitar', 'marathon', 'thesis', 'compiler', 'telescope', 'sourdough',
];

/**
 * Deterministic, varied reply text of approximately `chars` length.
 *
 * `seed` makes it reproducible per turn while still differing between turns.
 */
function buildReplyText(chars, seed) {
  let state = hash32(String(seed)) || 1;
  const next = () => {
    state ^= state << 13;
    state >>>= 0;
    state ^= state >> 17;
    state ^= state << 5;
    state >>>= 0;
    return state;
  };
  const words = [];
  let len = 0;
  while (len < chars) {
    const w = VOCAB[next() % VOCAB.length];
    words.push(w);
    len += w.length + 1;
  }
  return words.join(' ').slice(0, chars);
}

async function handleCompletion(req, res, body, opts) {
  let parsed;
  try {
    parsed = JSON.parse(body);
  } catch {
    stats.malformedRequests += 1;
    sendJson(res, 400, { error: { message: 'invalid JSON body' } });
    return;
  }

  const messages = Array.isArray(parsed.messages) ? parsed.messages : [];
  // Turn depth without stored state: every tool result already in the thread is
  // one tool call we previously emitted.
  const depth = messages.filter((m) => m?.role === 'tool').length;

  // Seed from stable request content so concurrent arrival order cannot change
  // which requests fail or how much latency they receive. Keep a per-content
  // attempt number so retries do not repeat the same injected failure forever.
  const requestKey = String(hash32(body));
  const attempt = nextRequestAttempt(requestKey);
  const seed = `${requestKey}:${attempt}`;

  if (opts.failRate > 0 && unitFrom(`fail:${seed}`) < opts.failRate) {
    stats.injectedFailures += 1;
    sendJson(res, 500, { error: { message: 'injected benchmark failure' } });
    return;
  }

  if (opts.latencyMs > 0 || opts.jitterMs > 0) {
    const offset = opts.jitterMs > 0 ? (unitFrom(`lat:${seed}`) * 2 - 1) * opts.jitterMs : 0;
    await sleep(Math.max(0, opts.latencyMs + offset));
  }

  stats.completions += 1;
  const model = typeof parsed.model === 'string' ? parsed.model : 'mock-model';
  const toolName = depth < opts.toolDepth ? pickTool(parsed.tools) : null;

  let message;
  let finishReason;
  if (toolName) {
    stats.toolCallsEmitted += 1;
    message = {
      role: 'assistant',
      content: null,
      tool_calls: [
        {
          id: `call_bench_${stats.completions}`,
          type: 'function',
          function: {
            name: toolName,
            // The core expects arguments as a JSON *string*.
            arguments: JSON.stringify(argumentsFor(toolName)),
          },
        },
      ],
    };
    finishReason = 'tool_calls';
  } else {
    stats.finalAnswers += 1;
    message = {
      role: 'assistant',
      content: buildReplyText(opts.replyChars, `${seed}:${stats.completions}`),
    };
    finishReason = 'stop';
  }

  // Rough but stable token accounting; the core records usage but does not
  // validate it against the text.
  const promptTokens = Math.max(1, Math.ceil(body.length / 4));
  const completionTokens = Math.max(1, Math.ceil(opts.replyChars / 4));

  sendJson(res, 200, {
    id: `chatcmpl-bench-${stats.completions}`,
    object: 'chat.completion',
    created: Math.floor(Date.now() / 1000),
    model,
    choices: [{ index: 0, message, finish_reason: finishReason }],
    usage: {
      prompt_tokens: promptTokens,
      completion_tokens: completionTokens,
      total_tokens: promptTokens + completionTokens,
    },
  });
}

/**
 * A deterministic pseudo-embedding derived from the input text.
 *
 * Returning one constant vector for every input would be simpler and is what
 * this did first — but it makes every cosine similarity identical, so retrieval
 * ranking becomes degenerate and any experiment about WHICH memories a change
 * surfaces is meaningless. Throughput measurements are unaffected either way
 * (the same work happens whatever the values are), so the flaw is invisible
 * unless you go looking for it.
 *
 * This derives a unit vector from a hash of the text: same text always yields
 * the same vector, similar-but-different texts yield different ones, and the
 * distribution is spread rather than collapsed onto a point. Not semantically
 * meaningful — nothing here models real language — but structurally realistic
 * enough to compare retrieval strategies against each other.
 */
function embeddingFor(text, dims) {
  // xorshift32 seeded by the content hash: cheap, deterministic, no shared state.
  let state = hash32(text) || 1;
  const next = () => {
    state ^= state << 13;
    state >>>= 0;
    state ^= state >> 17;
    state ^= state << 5;
    state >>>= 0;
    return state / 0xffffffff;
  };
  const vec = new Array(dims);
  let norm = 0;
  for (let i = 0; i < dims; i += 1) {
    // Box-Muller-ish spread around zero, so vectors are not all in one orthant.
    const v = next() * 2 - 1;
    vec[i] = v;
    norm += v * v;
  }
  norm = Math.sqrt(norm) || 1;
  for (let i = 0; i < dims; i += 1) vec[i] /= norm;
  return vec;
}

function handleEmbeddings(res, body, opts) {
  stats.embeddings += 1;
  let count = 1;
  let inputs = [''];
  try {
    const parsed = JSON.parse(body);
    if (Array.isArray(parsed.input)) {
      inputs = parsed.input.map((v) => (typeof v === 'string' ? v : JSON.stringify(v)));
      count = Math.max(1, inputs.length);
    } else if (typeof parsed.input === 'string') {
      inputs = [parsed.input];
    }
  } catch {
    stats.malformedRequests += 1;
  }
  // The dimension MUST match what the memory store expects, or every chunk is
  // stored without vectors and the memory write path runs degraded for the whole
  // run — silently, as a warning rather than an error. 1536 (the usual
  // text-embedding-3-small width) is the wrong default here; the store wants
  // 1024.
  sendJson(res, 200, {
    object: 'list',
    model: 'mock-embedding',
    data: Array.from({ length: count }, (_, i) => ({
      object: 'embedding',
      index: i,
      embedding: embeddingFor(inputs[i] ?? '', opts.embedDims),
    })),
    usage: { prompt_tokens: count, total_tokens: count },
  });
}

const opts = parseArgs(process.argv);

const server = http.createServer((req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${opts.port}`);
  const path = url.pathname;

  if (req.method === 'GET' && (path === '/health' || path === '/__bench/health')) {
    sendJson(res, 200, { ok: true });
    return;
  }
  if (req.method === 'GET' && path === '/__bench/stats') {
    sendJson(res, 200, { ...stats, uptimeMs: Date.now() - stats.startedAt });
    return;
  }
  if (req.method === 'POST' && path === '/__bench/reset') {
    for (const key of Object.keys(stats)) {
      if (key !== 'startedAt') stats[key] = 0;
    }
    sendJson(res, 200, { ok: true });
    return;
  }

  readBody(req)
    .then(async (body) => {
      if (req.method === 'POST' && path.endsWith('/chat/completions')) {
        await handleCompletion(req, res, body, opts);
        return;
      }
      if (req.method === 'POST' && path.endsWith('/embeddings')) {
        handleEmbeddings(res, body, opts);
        return;
      }
      if (req.method === 'POST' && path.includes('/telemetry/')) {
        stats.telemetry += 1;
        sendJson(res, 200, {});
        return;
      }
      const stub = ANCILLARY_ROUTES[`${req.method} ${path}`];
      if (stub) {
        stats.ancillary += 1;
        sendJson(res, 200, stub());
        return;
      }
      // Anything else is a route the core reached for that we did not
      // anticipate. Count it loudly — a rising number here means the benchmark
      // is silently exercising a degraded path.
      stats.unknownRoutes += 1;
      process.stderr.write(`[mock-llm] unhandled ${req.method} ${path}\n`);
      sendJson(res, 404, { error: { message: `unhandled route: ${path}` } });
    })
    .catch((err) => {
      stats.malformedRequests += 1;
      if (!res.headersSent) {
        sendJson(res, 400, { error: { message: String(err?.message ?? err) } });
      }
    });
});

// Long agent runs hold connections open; do not let Node time them out mid-turn.
server.keepAliveTimeout = 120_000;
server.headersTimeout = 125_000;

server.listen(opts.port, '127.0.0.1', () => {
  process.stderr.write(
    `[mock-llm] listening on http://127.0.0.1:${opts.port} ` +
      `(latency=${opts.latencyMs}±${opts.jitterMs}ms tool-depth=${opts.toolDepth} ` +
      `reply-chars=${opts.replyChars} fail-rate=${opts.failRate})\n`,
  );
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    server.close(() => process.exit(0));
  });
}
