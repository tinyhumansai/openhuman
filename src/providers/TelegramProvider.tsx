import {
  useEffect,
  useRef,
  createContext,
  useContext,
  ReactNode,
  useCallback,
} from "react";
import { useAppSelector, useAppDispatch } from "../store/hooks";
import {
  selectIsAuthenticated,
  selectIsInitialized,
  selectConnectionStatus,
  selectSessionString,
  selectTelegramCurrentUserId,
} from "../store/telegramSelectors";
import { initializeTelegram, connectTelegram } from "../store/telegram";
import { mtprotoService } from "../services/mtprotoService";

interface TelegramContextType {
  isAuthenticated: boolean;
  isInitialized: boolean;
  connectionStatus: "disconnected" | "connecting" | "connected" | "error";
  checkConnection: () => Promise<boolean>;
}

const TelegramContext = createContext<TelegramContextType | undefined>(
  undefined,
);

export const useTelegram = () => {
  const context = useContext(TelegramContext);
  if (!context) {
    throw new Error("useTelegram must be used within TelegramProvider");
  }
  return context;
};

interface TelegramProviderProps {
  children: ReactNode;
}

const MAX_RETRIES = 5;
const BASE_DELAY_MS = 1000;

/**
 * TelegramProvider manages the Telegram MTProto connection
 * - Initializes when app-authenticated (JWT), or has Telegram session / authenticated
 * - Starts init+connect in parallel with login (token) so connect modal is ready sooner
 * - Connects automatically with exponential backoff on failure
 * - Provides Telegram context to children
 */
const TelegramProvider = ({ children }: TelegramProviderProps) => {
  const dispatch = useAppDispatch();
  const token = useAppSelector((state) => state.auth.token);
  const userId = useAppSelector(selectTelegramCurrentUserId);
  const isAuthenticated = useAppSelector(selectIsAuthenticated);
  const isInitialized = useAppSelector(selectIsInitialized);
  const connectionStatus = useAppSelector(selectConnectionStatus);
  const sessionString = useAppSelector(selectSessionString);

  const setupInProgressRef = useRef(false);
  const setupCompleteRef = useRef(false);
  const retryCountRef = useRef(0);
  const retryTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const hasSession = !!sessionString;
  const shouldSetupTelegram = !!token && !!userId;

  const clearRetryTimeout = useCallback(() => {
    if (retryTimeoutRef.current !== null) {
      clearTimeout(retryTimeoutRef.current);
      retryTimeoutRef.current = null;
    }
  }, []);

  // Initialize and connect Telegram with exponential backoff on failure
  useEffect(() => {
    if (!shouldSetupTelegram) {
      // Reset all state when conditions no longer met
      setupInProgressRef.current = false;
      setupCompleteRef.current = false;
      retryCountRef.current = 0;
      clearRetryTimeout();
      return;
    }

    // If setup is already complete and everything is connected, don't run again
    if (
      setupCompleteRef.current &&
      isInitialized &&
      mtprotoService.isReady() &&
      connectionStatus === "connected"
    ) {
      return;
    }

    if (setupInProgressRef.current) {
      return;
    }

    const setupTelegram = async () => {
      setupInProgressRef.current = true;

      try {
        // Stale-state guard: Redux says isInitialized but the service has no client
        // (happens after page reload — persist restores isInitialized: true but mtprotoService starts fresh)
        const needsInit = !mtprotoService.isReady();

        if (needsInit) {
          await dispatch(initializeTelegram(userId)).unwrap();
          // After init, let the effect re-fire to handle connect
          setupInProgressRef.current = false;
          return;
        }

        if (connectionStatus !== "connected") {
          await dispatch(connectTelegram(userId)).unwrap();
          setupInProgressRef.current = false;
          return;
        }

        // Setup complete
        setupInProgressRef.current = false;
        setupCompleteRef.current = true;
        retryCountRef.current = 0;
      } catch (error) {
        console.error("Failed to setup Telegram:", error);
        setupInProgressRef.current = false;
        setupCompleteRef.current = false;

        // Exponential backoff
        retryCountRef.current += 1;
        if (retryCountRef.current <= MAX_RETRIES) {
          const delay = BASE_DELAY_MS * Math.pow(2, retryCountRef.current - 1);
          console.log(
            `Telegram setup retry ${retryCountRef.current}/${MAX_RETRIES} in ${delay}ms`,
          );
          clearRetryTimeout();
          retryTimeoutRef.current = setTimeout(() => {
            retryTimeoutRef.current = null;
            // Re-trigger by calling setupTelegram again
            // The effect deps won't have changed, so we call directly
            setupTelegram();
          }, delay);
        } else {
          console.error(
            `Telegram setup failed after ${MAX_RETRIES} retries. Giving up.`,
          );
        }
      }
    };

    setupTelegram();

    return () => {
      clearRetryTimeout();
    };
  }, [
    shouldSetupTelegram,
    isInitialized,
    connectionStatus,
    dispatch,
    userId,
    clearRetryTimeout,
  ]);

  // Check connection status once when Telegram reports as connected
  useEffect(() => {
    if (
      !shouldSetupTelegram ||
      !isInitialized ||
      connectionStatus !== "connected"
    ) {
      return;
    }

    // Only check if the service is actually ready
    if (!mtprotoService.isReady()) return;
    checkConnection();
  }, [shouldSetupTelegram, isInitialized, connectionStatus, userId]);

  const checkConnection = async (): Promise<boolean> => {
    try {
      return await mtprotoService.checkConnection(userId || undefined);
    } catch (error) {
      console.warn("Connection check failed:", error);
      return false;
    }
  };

  const value: TelegramContextType = {
    isAuthenticated: isAuthenticated || hasSession,
    isInitialized,
    connectionStatus,
    checkConnection,
  };

  return (
    <TelegramContext.Provider value={value}>
      {children}
    </TelegramContext.Provider>
  );
};

export default TelegramProvider;
