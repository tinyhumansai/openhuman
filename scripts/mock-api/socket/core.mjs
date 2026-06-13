import { json, setCors } from "../http.mjs";
import {
  appendSocketEvent,
  attachWebSocketToSession,
  behavior,
  buildSocketReadyPayload,
  createMockId,
  drainSocketPackets,
  getSocketSession,
  listSocketSessions,
  parseBehaviorJson,
  queueSocketPacket,
  registerSocketSession,
  touchSocketSession,
  dropSocketSession,
} from "../state.mjs";
import {
  decodePollingPayload,
  engineOpenPacket,
  normalizeAuthPayload,
  parseRequestUrl,
  socketConnectErrorPacket,
  socketConnectPacket,
  socketEventPacket,
  encodePollingPayload,
} from "./protocol.mjs";
import {
  acceptWebSocket,
  closeWebSocket,
  decodeWebSocketFrames,
  sendWsText,
  socketIsOpen,
  upgradeWebSocket,
} from "./websocket.mjs";

function socketIoSid() {
  return `mock-sio-${createMockId("sid")}`;
}

function authenticateSession(authPayload) {
  const mockBehavior = behavior();
  const socketAuthMode = mockBehavior.socketAuthMode || "required";
  const token =
    authPayload && typeof authPayload === "object"
      ? authPayload.token
      : undefined;

  if (socketAuthMode !== "disabled" && !token) {
    return { ok: false, message: "No token provided" };
  }
  if (
    mockBehavior.socketAuthMode === "reject" ||
    mockBehavior.socketReject === "true" ||
    String(token || "").includes("invalid")
  ) {
    return {
      ok: false,
      message: mockBehavior.socketRejectMessage || "Authentication failed",
    };
  }

  return {
    ok: true,
    token: token || null,
    userId: mockBehavior.socketUserId || mockBehavior.userId || "mock-user",
  };
}

function logSocketCheckpoint(kind, detail = {}) {
  appendSocketEvent({ direction: "system", kind, ...detail });
}

function writePollingResponse(res, packets) {
  setCors(res);
  res.writeHead(200, { "Content-Type": "text/plain; charset=UTF-8" });
  res.end(encodePollingPayload(packets));
}

function writePollingOk(res) {
  setCors(res);
  res.writeHead(200, { "Content-Type": "text/plain; charset=UTF-8" });
  res.end("ok");
}

function sendSocketPacket(session, packet) {
  const target = getSocketSession(session.sid);
  if (!target) return false;
  target.lastSeenAt = new Date().toISOString();
  if (socketIsOpen(target.webSocket) && target.upgradedToWebSocket === true) {
    sendWsText(target.webSocket, packet);
    return true;
  }
  return queueSocketPacket(target.sid, packet);
}

function cleanupRejectedSession(session) {
  const live = getSocketSession(session.sid);
  if (!live) return;
  if (socketIsOpen(live.webSocket)) {
    closeWebSocket(live.webSocket);
    dropSocketSession(live.sid);
    return;
  }
  touchSocketSession(live.sid, { disconnectAfterDrain: true });
}

function scheduleMockSocketActions(session, actions = []) {
  for (const action of actions) {
    const delayMs = Math.max(0, Number(action?.delayMs || 0));
    setTimeout(() => {
      if (action?.disconnect === true) {
        disconnectMockSockets({ targetSid: session.sid });
        return;
      }
      if (typeof action?.event === "string" && action.event) {
        emitMockSocketEvent({
          event: action.event,
          data:
            action.data === "__ready__"
              ? buildSocketReadyPayload(session)
              : (action.data ?? null),
          targetSid: session.sid,
        });
        return;
      }
      if (action?.agentAudioStream) {
        emitMockAgentAudioStream({
          targetSid: session.sid,
          ...action.agentAudioStream,
        });
      }
    }, delayMs);
  }
}

function onSocketConnected(session) {
  touchSocketSession(session.sid, { connected: true });
  sendSocketPacket(session, socketConnectPacket(session));
  sendSocketPacket(
    session,
    socketEventPacket("ready", buildSocketReadyPayload(session)),
  );

  logSocketCheckpoint("connected", {
    sid: session.sid,
    socketId: session.socketId,
    userId: session.userId,
    transport: session.transport,
  });

  const connectScript = parseBehaviorJson("socketConnectScript", []);
  if (Array.isArray(connectScript) && connectScript.length > 0) {
    scheduleMockSocketActions(session, connectScript);
  }
}

function handleClientSocketEvent(session, event, data) {
  appendSocketEvent({
    direction: "inbound",
    kind: "event",
    sid: session.sid,
    socketId: session.socketId,
    userId: session.userId,
    event,
    data,
  });

  if (event === "webhook:response" && data?.correlationId) {
    sendSocketPacket(
      session,
      socketEventPacket(`webhook:response:${data.correlationId}`, data),
    );
  }

  const scripts = parseBehaviorJson("socketClientEventScripts", {});
  const actions = Array.isArray(scripts?.[event]) ? scripts[event] : [];
  if (actions.length > 0) {
    scheduleMockSocketActions(session, actions);
  }
}

function handleSocketPacket(session, packet) {
  touchSocketSession(session.sid);

  if (packet === "2") {
    sendSocketPacket(session, "3");
    return;
  }

  if (packet === "2probe") {
    sendSocketPacket(session, "3probe");
    return;
  }

  if (packet === "5") {
    touchSocketSession(session.sid, {
      upgradedToWebSocket: true,
      transport: "websocket",
    });
    logSocketCheckpoint("upgrade_complete", { sid: session.sid });
    const pending = drainSocketPackets(session.sid);
    const live = getSocketSession(session.sid);
    if (live?.webSocket && pending.length > 0) {
      for (const queued of pending) {
        sendWsText(live.webSocket, queued);
      }
    }
    return;
  }

  if (packet.startsWith("40")) {
    const auth = normalizeAuthPayload(packet);
    const result = authenticateSession(auth);
    if (!result.ok) {
      sendSocketPacket(session, socketConnectErrorPacket(result.message));
      appendSocketEvent({
        direction: "outbound",
        kind: "connect_error",
        sid: session.sid,
        message: result.message,
      });
      cleanupRejectedSession(session);
      return;
    }
    touchSocketSession(session.sid, {
      token: result.token,
      userId: result.userId,
    });
    onSocketConnected(getSocketSession(session.sid));
    return;
  }

  if (packet.startsWith("42")) {
    try {
      const payload = JSON.parse(packet.slice(2));
      const [event, data] = Array.isArray(payload) ? payload : [];
      if (typeof event === "string" && event) {
        handleClientSocketEvent(session, event, data);
      }
    } catch {
      appendSocketEvent({
        direction: "system",
        kind: "parse_error",
        sid: session.sid,
        packet,
      });
    }
  }
}

function createSession(transport) {
  const sid = socketIoSid();
  const session = registerSocketSession({
    sid,
    socketId: sid,
    transport,
    createdAt: new Date().toISOString(),
  });
  logSocketCheckpoint("session_created", { sid, transport });
  return session;
}

function lookupSessionFromUrl(urlObj) {
  const sid = urlObj.searchParams.get("sid");
  if (!sid) return null;
  return getSocketSession(sid);
}

function matchSession(session, filters = {}) {
  if (filters.targetSid && session.sid !== filters.targetSid) return false;
  if (filters.excludeSid && session.sid === filters.excludeSid) return false;
  if (filters.targetUserId && session.userId !== filters.targetUserId) {
    return false;
  }
  return true;
}

export function handleSocketRequest(ctx) {
  const { method, url, body, res } = ctx;
  if (!url?.startsWith("/socket.io/")) return false;

  const urlObj = parseRequestUrl(url);
  const transport = urlObj.searchParams.get("transport");
  if (transport !== "polling") {
    json(res, 400, {
      success: false,
      error: "Mock socket only handles polling HTTP",
    });
    return true;
  }

  if (method === "GET") {
    const existing = lookupSessionFromUrl(urlObj);
    if (!existing) {
      const session = createSession("polling");
      writePollingResponse(res, [engineOpenPacket(session.sid)]);
      return true;
    }

    const packets = drainSocketPackets(existing.sid);
    if (existing.disconnectAfterDrain === true) {
      dropSocketSession(existing.sid);
    }
    writePollingResponse(res, packets.length > 0 ? packets : ["6"]);
    return true;
  }

  if (method === "POST") {
    const session = lookupSessionFromUrl(urlObj);
    if (!session) {
      json(res, 400, { success: false, error: "Unknown socket session" });
      return true;
    }
    const packets = decodePollingPayload(body);
    for (const packet of packets) {
      handleSocketPacket(session, packet);
    }
    writePollingOk(res);
    return true;
  }

  json(res, 405, { success: false, error: "Method not allowed" });
  return true;
}

function attachAcceptedWebSocket(req, socket) {
  const urlObj = parseRequestUrl(req.url);
  const requestedSid = urlObj.searchParams.get("sid");
  let session = requestedSid ? getSocketSession(requestedSid) : null;

  if (!session) {
    session = createSession("websocket");
    attachWebSocketToSession(session.sid, socket, {
      transport: "websocket",
      upgraded: true,
    });
    sendWsText(socket, engineOpenPacket(session.sid, []));
  } else {
    attachWebSocketToSession(session.sid, socket, { upgraded: false });
  }

  logSocketCheckpoint("websocket_attached", { sid: session.sid, requestedSid });

  decodeWebSocketFrames(socket, (packet) =>
    handleSocketPacket(session, packet),
  );

  socket.on("close", () => {
    const live = getSocketSession(session.sid);
    if (live?.upgradedToWebSocket === true) {
      dropSocketSession(session.sid);
    } else if (live) {
      touchSocketSession(session.sid, {
        webSocket: null,
        transport: "polling",
      });
    }
    logSocketCheckpoint("websocket_closed", { sid: session.sid });
  });
  socket.on("error", () => {});
}

export function handleWebSocketUpgrade(req, socket, head) {
  if (!req.url?.startsWith("/socket.io/")) {
    socket.destroy();
    return;
  }

  if (
    upgradeWebSocket(req, socket, head, (ws) =>
      attachAcceptedWebSocket(req, ws),
    )
  ) {
    return;
  }

  if (!acceptWebSocket(req, socket)) return;
  attachAcceptedWebSocket(req, socket);
}

export function emitMockSocketEvent({
  event,
  data,
  targetSid,
  targetUserId,
  excludeSid,
  delayMs = 0,
}) {
  if (typeof event !== "string" || !event) return 0;

  const matchingSessions = listSocketSessions().filter((session) =>
    matchSession(session, { targetSid, targetUserId, excludeSid }),
  );

  const deliver = () => {
    for (const info of matchingSessions) {
      const session = getSocketSession(info.sid);
      if (!session) continue;
      sendSocketPacket(session, socketEventPacket(event, data));
      appendSocketEvent({
        direction: "outbound",
        kind: "event",
        sid: session.sid,
        socketId: session.socketId,
        userId: session.userId,
        event,
        data,
      });
    }
  };

  const normalizedDelay = Math.max(0, Number(delayMs || 0));
  if (normalizedDelay > 0) {
    setTimeout(deliver, normalizedDelay);
  } else {
    deliver();
  }

  return matchingSessions.length;
}

export function emitMockAgentAudioStream({
  sessionId = "mock-agent-session",
  requestId,
  text = "",
  voiceId = "",
  contentType = "audio/mpeg",
  chunks,
  chunkDelayMs = 0,
  targetSid,
  targetUserId,
  excludeSid,
}) {
  const resolvedRequestId = requestId || `mock-audio-${createMockId("req")}`;
  const normalizedChunks =
    Array.isArray(chunks) && chunks.length > 0
      ? chunks
      : [Buffer.from("ID3MOCKAUDIO", "utf8").toString("base64")];

  let delivered = emitMockSocketEvent({
    event: "agent:audio:start",
    data: {
      sessionId,
      requestId: resolvedRequestId,
      contentType,
      voiceId,
      text,
    },
    targetSid,
    targetUserId,
    excludeSid,
  });

  normalizedChunks.forEach((chunk, index) => {
    delivered = Math.max(
      delivered,
      emitMockSocketEvent({
        event: "agent:audio:chunk",
        data: {
          sessionId,
          requestId: resolvedRequestId,
          chunk,
        },
        targetSid,
        targetUserId,
        excludeSid,
        delayMs: chunkDelayMs * (index + 1),
      }),
    );
  });

  delivered = Math.max(
    delivered,
    emitMockSocketEvent({
      event: "agent:audio:end",
      data: {
        sessionId,
        requestId: resolvedRequestId,
        ttsCharCount: String(text || "").length,
      },
      targetSid,
      targetUserId,
      excludeSid,
      delayMs: chunkDelayMs * (normalizedChunks.length + 1),
    }),
  );

  return delivered;
}

export function disconnectMockSockets({ targetSid, targetUserId } = {}) {
  let disconnected = 0;
  for (const sessionInfo of listSocketSessions()) {
    if (!matchSession(sessionInfo, { targetSid, targetUserId })) continue;
    const session = getSocketSession(sessionInfo.sid);
    if (!session) continue;
    closeWebSocket(session.webSocket);
    dropSocketSession(session.sid);
    disconnected += 1;
  }
  return disconnected;
}
