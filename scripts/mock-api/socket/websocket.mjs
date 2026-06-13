import crypto from "node:crypto";
import { WebSocketServer } from "ws";

const wsServer = new WebSocketServer({ noServer: true });

function isWsLibrarySocket(socket) {
  return socket && typeof socket.send === "function";
}

export function socketIsOpen(socket) {
  if (!socket) return false;
  if (isWsLibrarySocket(socket)) return socket.readyState === 1;
  return !socket.destroyed;
}

export function closeWebSocket(socket) {
  if (!socket) return;
  try {
    if (isWsLibrarySocket(socket)) {
      socket.close();
      socket.terminate?.();
      return;
    }
    socket.end?.();
    socket.destroy?.();
  } catch {
    // noop
  }
}

export function sendWsFrame(socket, opcode, payload) {
  if (!socket || socket.destroyed) return;

  const len = payload.length;
  let header;
  if (len < 126) {
    header = Buffer.alloc(2);
    header[0] = 0x80 | opcode;
    header[1] = len;
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }

  try {
    socket.write(header);
    socket.write(payload);
  } catch {
    // noop
  }
}

export function sendWsText(socket, text) {
  if (isWsLibrarySocket(socket)) {
    if (socket.readyState === 1) socket.send(String(text));
    return;
  }
  sendWsFrame(socket, 0x01, Buffer.from(text, "utf-8"));
}

export function upgradeWebSocket(req, socket, head, onConnection) {
  if (!Buffer.isBuffer(head)) return false;
  wsServer.handleUpgrade(req, socket, head, (ws) => {
    onConnection(ws);
  });
  return true;
}

export function acceptWebSocket(req, socket) {
  const rawKey = req.headers["sec-websocket-key"];
  const key = Array.isArray(rawKey) ? rawKey[0]?.trim() : rawKey?.trim();
  if (!key) {
    socket.destroy();
    return false;
  }
  const acceptKey = crypto
    .createHash("sha1")
    .update(key + "258EAFA5-E914-47DA-95CA-5AB5DC085B11")
    .digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\n" +
      "Upgrade: websocket\r\n" +
      "Connection: Upgrade\r\n" +
      `Sec-WebSocket-Accept: ${acceptKey}\r\n` +
      "\r\n",
  );
  return true;
}

export function decodeWebSocketFrames(socket, onText) {
  if (isWsLibrarySocket(socket)) {
    socket.on("message", (data) => {
      onText(Buffer.isBuffer(data) ? data.toString("utf-8") : String(data));
    });
    return;
  }

  let buffer = Buffer.alloc(0);

  socket.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    while (buffer.length >= 2) {
      const firstByte = buffer[0];
      const opcode = firstByte & 0x0f;
      const secondByte = buffer[1];
      const masked = (secondByte & 0x80) !== 0;
      let payloadLen = secondByte & 0x7f;
      let offset = 2;

      if (payloadLen === 126) {
        if (buffer.length < 4) return;
        payloadLen = buffer.readUInt16BE(2);
        offset = 4;
      } else if (payloadLen === 127) {
        if (buffer.length < 10) return;
        payloadLen = Number(buffer.readBigUInt64BE(2));
        offset = 10;
      }

      const maskSize = masked ? 4 : 0;
      const totalLen = offset + maskSize + payloadLen;
      if (buffer.length < totalLen) return;

      let payload = buffer.subarray(offset + maskSize, totalLen);
      if (masked) {
        const mask = buffer.subarray(offset, offset + 4);
        payload = Buffer.from(payload);
        for (let i = 0; i < payload.length; i += 1) {
          payload[i] ^= mask[i % 4];
        }
      }

      buffer = buffer.subarray(totalLen);

      if (opcode === 0x08) {
        socket.end();
        return;
      }

      if (opcode === 0x09) {
        sendWsFrame(socket, 0x0a, payload);
        continue;
      }

      if (opcode === 0x01) {
        onText(payload.toString("utf-8"));
      }
    }
  });
}
