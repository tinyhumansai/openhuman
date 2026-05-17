import { createRequire } from "node:module";

const requireFromApp = createRequire(
  new URL("../../../app/package.json", import.meta.url),
);

export const { io: SocketClient } = requireFromApp("socket.io-client");

export function onceSocket(socket, event) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`Timed out waiting for socket event: ${event}`));
    }, 5_000);

    const onEvent = (...args) => {
      cleanup();
      resolve(args[0]);
    };

    const onError = (err) => {
      cleanup();
      reject(err instanceof Error ? err : new Error(String(err)));
    };

    const cleanup = () => {
      clearTimeout(timeout);
      socket.off(event, onEvent);
      if (event !== "connect_error") {
        socket.off("connect_error", onError);
      }
    };

    socket.on(event, onEvent);
    if (event !== "connect_error") {
      socket.on("connect_error", onError);
    }
  });
}

export function createSocket(baseUrl, options = {}) {
  return SocketClient(baseUrl, {
    path: "/socket.io/",
    reconnection: false,
    forceNew: true,
    timeout: 3_000,
    ...options,
  });
}
