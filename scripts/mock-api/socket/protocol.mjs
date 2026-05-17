export const EIO_PING_INTERVAL = 25_000;
export const EIO_PING_TIMEOUT = 20_000;
export const EIO_MAX_PAYLOAD = 1_000_000;
export const POLLING_SEPARATOR = "\x1e";

export function parseRequestUrl(rawUrl) {
  return new URL(rawUrl || "/socket.io/", "http://127.0.0.1");
}

export function engineOpenPacket(sid, upgrades = ["websocket"]) {
  return `0${JSON.stringify({
    sid,
    upgrades,
    pingInterval: EIO_PING_INTERVAL,
    pingTimeout: EIO_PING_TIMEOUT,
    maxPayload: EIO_MAX_PAYLOAD,
  })}`;
}

export function socketConnectPacket(session) {
  return `40${JSON.stringify({ sid: session.socketId })}`;
}

export function socketConnectErrorPacket(message) {
  return `44${JSON.stringify({ message })}`;
}

export function socketEventPacket(event, data) {
  const payload = data === undefined ? [event] : [event, data];
  return `42${JSON.stringify(payload)}`;
}

export function encodePollingPayload(packets) {
  return packets.join(POLLING_SEPARATOR);
}

export function decodePollingPayload(rawBody) {
  const body = String(rawBody || "");
  if (!body) return [];
  return body.split(POLLING_SEPARATOR).filter(Boolean);
}

export function normalizeAuthPayload(packet) {
  if (!packet.startsWith("40")) return null;
  const payload = packet.slice(2).trim();
  if (!payload) return {};
  try {
    return JSON.parse(payload);
  } catch {
    return {};
  }
}
