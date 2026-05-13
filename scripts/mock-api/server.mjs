import http from "node:http";

import { handleAdmin } from "./admin.mjs";
import {
  json,
  normalizeHeaders,
  readBody,
  requestOrigin,
  setCors,
  tryParseJson,
} from "./http.mjs";
import { handleAuth } from "./routes/auth.mjs";
import { handleConversations } from "./routes/conversations.mjs";
import { handleCron } from "./routes/cron.mjs";
import { handleIntegrations } from "./routes/integrations.mjs";
import { handleInvites } from "./routes/invites.mjs";
import { handleOAuth } from "./routes/oauth.mjs";
import { handlePayments } from "./routes/payments.mjs";
import { handleUser } from "./routes/user.mjs";
import { handleVersion } from "./routes/version.mjs";
import { handleWebhooks } from "./routes/webhooks.mjs";
import { handleEnginePollingOpen, handleWebSocketUpgrade } from "./socket.mjs";
import {
  appendRequest,
  DEFAULT_PORT,
  MAX_PORT_RETRY_ATTEMPTS,
  openSockets,
} from "./state.mjs";

let server = null;

// Order matters: admin & socket.io short-circuit early; the rest fall through
// in domain order so the cheapest predicates run first.
const ROUTE_HANDLERS = [
  handleOAuth,
  handleAuth,
  handleUser,
  handleInvites,
  handlePayments,
  handleIntegrations,
  handleWebhooks,
  handleCron,
  handleConversations,
  handleVersion,
];

async function handleRequest(req, res) {
  const method = req.method ?? "GET";
  const url = req.url ?? "/";
  const body = await readBody(req);
  const parsedBody = tryParseJson(body);
  const origin = requestOrigin(req);

  appendRequest({
    method,
    url,
    body,
    headers: normalizeHeaders(req.headers),
    timestamp: Date.now(),
  });

  if (method === "OPTIONS") {
    setCors(res);
    res.writeHead(204);
    res.end();
    return;
  }

  const ctx = {
    method,
    url,
    body,
    parsedBody,
    origin,
    req,
    res,
    getPort: getMockServerPort,
  };

  if (handleAdmin(ctx)) return;

  if (url.startsWith("/socket.io/")) {
    handleEnginePollingOpen(req, res);
    return;
  }

  for (const handler of ROUTE_HANDLERS) {
    if (await handler(ctx)) return;
  }

  // Catch-all: fail fast so tests notice missing mock endpoints.
  console.log(`[MockServer] UNHANDLED ${method} ${url}`);
  json(res, 404, {
    success: false,
    error: `Mock server: no handler for ${method} ${url}`,
  });
}

export function getMockServerPort() {
  const address = server?.address();
  return typeof address === "object" && address ? address.port : null;
}

function createServerInstance() {
  const nextServer = http.createServer((req, res) => {
    handleRequest(req, res).catch((err) => {
      console.error("[MockServer] Unhandled error:", err);
      json(res, 500, { success: false, error: "Internal mock error" });
    });
  });
  nextServer.on("connection", (socket) => {
    openSockets.add(socket);
    socket.on("close", () => openSockets.delete(socket));
  });
  nextServer.on("upgrade", (req, socket) => handleWebSocketUpgrade(req, socket));
  return nextServer;
}

function listen(serverInstance, port) {
  return new Promise((resolve, reject) => {
    const onError = (err) => {
      serverInstance.off("listening", onListening);
      reject(err);
    };
    const onListening = () => {
      serverInstance.off("error", onError);
      const address = serverInstance.address();
      const resolvedPort =
        typeof address === "object" && address ? address.port : port;
      resolve(resolvedPort);
    };
    serverInstance.once("error", onError);
    serverInstance.once("listening", onListening);
    serverInstance.listen(port, "127.0.0.1");
  });
}

export async function startMockServer(port = DEFAULT_PORT, options = {}) {
  if (server) {
    return { port: getMockServerPort() ?? port, alreadyRunning: true };
  }

  const preferredPort =
    Number.isInteger(port) && port > 0 ? port : DEFAULT_PORT;
  const retryIfInUse = options.retryIfInUse === true;
  const candidatePorts = retryIfInUse
    ? [
        preferredPort,
        ...Array.from(
          { length: MAX_PORT_RETRY_ATTEMPTS },
          (_, i) => preferredPort + i + 1,
        ),
        0,
      ]
    : [preferredPort];

  let lastError = null;
  for (const candidatePort of candidatePorts) {
    const nextServer = createServerInstance();
    try {
      const resolvedPort = await listen(nextServer, candidatePort);
      server = nextServer;
      const retryNote =
        resolvedPort === preferredPort
          ? ""
          : ` (preferred ${preferredPort} unavailable)`;
      console.log(
        `[MockServer] Listening on http://127.0.0.1:${resolvedPort}${retryNote}`,
      );
      return {
        port: resolvedPort,
        alreadyRunning: false,
        requestedPort: preferredPort,
        retried: resolvedPort !== preferredPort,
      };
    } catch (err) {
      try {
        nextServer.close();
      } catch {
        // The failed candidate may never have reached the listening state.
      }
      lastError = err;
      if (!retryIfInUse || err?.code !== "EADDRINUSE") {
        throw err;
      }
      console.warn(
        `[MockServer] Port ${candidatePort} unavailable; trying another local port`,
      );
    }
  }

  throw lastError ?? new Error("Mock server failed to start");
}

export function stopMockServer() {
  return new Promise((resolve) => {
    if (!server) {
      resolve();
      return;
    }
    for (const socket of openSockets) {
      socket.destroy();
    }
    openSockets.clear();
    server.close(() => {
      console.log("[MockServer] Stopped");
      server = null;
      resolve();
    });
  });
}
