import { useEffect, useRef } from "react";
import { useAppSelector } from "../store/hooks";
import { store } from "../store";
import { selectSocketStatus } from "../store/socketSelectors";
import { socketService } from "../services/socketService";
import {
  initTelegramMCPServer,
  getTelegramMCPServer,
  updateTelegramMCPServerSocket,
  cleanupTelegramMCPServer,
} from "../lib/mcp/telegram";
import {
  isTauri,
  setupTauriSocketListeners,
  cleanupTauriSocketListeners,
  reportSocketConnected,
  reportSocketDisconnected,
  reportSocketError,
  updateSocketStatus,
} from "../utils/tauriSocket";

/**
 * SocketProvider manages the socket connection based on JWT token
 * - Connects when token is set
 * - Disconnects when token is unset
 * - Integrates with Tauri for background persistence
 */
const SocketProvider = ({ children }: { children: React.ReactNode }) => {
  const token = useAppSelector((state) => state.auth.token);
  const socketStatus = useAppSelector(selectSocketStatus);
  const previousTokenRef = useRef<string | null>(null);
  const tauriListenersSetup = useRef(false);

  // Setup Tauri event listeners once
  useEffect(() => {
    if (isTauri() && !tauriListenersSetup.current) {
      setupTauriSocketListeners();
      tauriListenersSetup.current = true;
    }

    return () => {
      if (isTauri() && tauriListenersSetup.current) {
        cleanupTauriSocketListeners();
        tauriListenersSetup.current = false;
      }
    };
  }, []);

  // Handle socket connection based on token
  useEffect(() => {
    const previousToken = previousTokenRef.current;

    // Token was set - connect
    if (token && token !== previousToken) {
      socketService.connect(token);
      previousTokenRef.current = token;

      // Report to Rust that we're connecting
      if (isTauri()) {
        updateSocketStatus("connecting");
      }
    }

    // Token was unset - disconnect
    if (!token && previousToken) {
      socketService.disconnect();
      cleanupTelegramMCPServer();
      previousTokenRef.current = null;

      // Report to Rust
      if (isTauri()) {
        reportSocketDisconnected();
      }
    }
  }, [token]);

  // Handle MCP initialization and Tauri status reporting
  useEffect(() => {
    if (socketStatus === "connected") {
      const socket = socketService.getSocket();
      const server = getTelegramMCPServer();

      if (server) {
        updateTelegramMCPServerSocket(socket);
      } else {
        initTelegramMCPServer(socket);
      }

      // Report to Rust
      if (isTauri()) {
        reportSocketConnected(socket?.id);
      }
    } else if (socketStatus === "disconnected") {
      cleanupTelegramMCPServer();

      // Report to Rust
      if (isTauri()) {
        reportSocketDisconnected();
      }
    } else if (socketStatus === "connecting") {
      // Report connecting status to Rust
      if (isTauri()) {
        updateSocketStatus("connecting");
      }
    }
  }, [socketStatus]);

  // Listen for socket errors and report to Rust
  useEffect(() => {
    const socket = socketService.getSocket();
    if (!socket) return;

    const handleError = (error: Error) => {
      if (isTauri()) {
        reportSocketError(error.message || "Socket error");
      }
    };

    const handleConnectError = (error: Error) => {
      if (isTauri()) {
        reportSocketError(error.message || "Connection error");
        updateSocketStatus("error");
      }
    };

    socket.on("error", handleError);
    socket.on("connect_error", handleConnectError);

    return () => {
      socket.off("error", handleError);
      socket.off("connect_error", handleConnectError);
    };
  }, [socketStatus]);

  // Cleanup on unmount only
  useEffect(() => {
    return () => {
      // Only disconnect on actual unmount (e.g., app closing)
      // Don't disconnect on re-renders or route changes
      const currentToken = store.getState().auth.token;
      if (!currentToken) {
        socketService.disconnect();
      }
    };
  }, []); // Empty deps - only run cleanup on unmount

  return <>{children}</>;
};

export default SocketProvider;
