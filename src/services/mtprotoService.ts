import { TelegramClient } from "telegram";
import { StringSession } from "telegram/sessions";
import type { UserAuthParams, BotAuthParams } from "telegram/client/auth";
import { FloodWaitError } from "telegram/errors";
import { TELEGRAM_API_ID, TELEGRAM_API_HASH } from "../utils/config";
import { store } from "../store";
import { setSessionString } from "../store/telegram";

type LoginOptions = UserAuthParams | BotAuthParams;

/** Status events emitted during QR login for UI consumption */
export type QrLoginStatus =
  | { type: "token"; token: Buffer; expires: number; serverTime: number }
  | { type: "scanning_complete" }
  | { type: "dc_migration"; dcId: number }
  | { type: "2fa_required"; hint?: string }
  | { type: "success" }
  | { type: "error"; error: Error };

class MTProtoService {
  private static instance: MTProtoService | undefined;
  private client: TelegramClient | undefined;
  private isInitialized = false;
  private isConnected = false;
  private sessionString = "";
  private userId: string | null = null;
  private readonly apiId: number;
  private readonly apiHash: string;

  // In-flight promise guards — concurrent callers await the same promise
  private initializePromise: Promise<void> | null = null;
  private connectPromise: Promise<void> | null = null;
  private checkConnectionPromise: Promise<boolean> | null = null;

  // QR login race condition guard
  private isScanningComplete = false;

  private constructor() {
    // Private constructor to enforce singleton
    // Load API credentials from config once
    if (!TELEGRAM_API_ID || !TELEGRAM_API_HASH) {
      throw new Error(
        "TELEGRAM_API_ID and TELEGRAM_API_HASH must be configured",
      );
    }
    this.apiId = TELEGRAM_API_ID;
    this.apiHash = TELEGRAM_API_HASH;
  }

  static getInstance(): MTProtoService {
    if (!MTProtoService.instance) {
      MTProtoService.instance = new MTProtoService();
    }
    return MTProtoService.instance;
  }

  /**
   * Initialize the MTProto client with API credentials.
   * Session is stored in Redux (telegram.byUser[userId].sessionString).
   * Concurrent calls for the same userId await the same in-flight promise.
   */
  async initialize(userId: string): Promise<void> {
    if (this.isInitialized && this.client && this.userId === userId) {
      return;
    }
    // If already in-flight for the same user, deduplicate
    if (this.initializePromise && this.userId === userId) {
      return this.initializePromise;
    }

    this.initializePromise = this._doInitialize(userId).finally(() => {
      this.initializePromise = null;
    });
    return this.initializePromise;
  }

  private async _doInitialize(userId: string): Promise<void> {
    if (this.isInitialized && this.userId !== null && this.userId !== userId) {
      await this.clearSessionAndDisconnect(this.userId);
    }

    this.userId = userId;
    const sessionString = this.loadSession() || "";

    try {
      const stringSession = new StringSession(sessionString);
      this.sessionString = sessionString;

      this.client = new TelegramClient(
        stringSession,
        this.apiId,
        this.apiHash,
        {
          connectionRetries: 5,
          requestRetries: 5,
          floodSleepThreshold: 60, // Auto-retry FLOOD_WAIT errors up to 60 seconds
        },
      );

      this.isInitialized = true;
      console.log("MTProto client initialized successfully");
    } catch (error) {
      console.error("Failed to initialize MTProto client:", error);
      throw error;
    }
  }

  /**
   * Connect to Telegram servers.
   * Concurrent calls await the same in-flight promise.
   */
  async connect(): Promise<void> {
    if (!this.client) {
      throw new Error(
        "MTProto client not initialized. Call initialize() first.",
      );
    }

    if (this.isConnected) {
      return;
    }

    if (this.connectPromise) {
      return this.connectPromise;
    }

    this.connectPromise = this._doConnect().finally(() => {
      this.connectPromise = null;
    });
    return this.connectPromise;
  }

  private async _doConnect(): Promise<void> {
    try {
      await this.client!.connect();
      this.isConnected = true;
      console.log("Connected to Telegram successfully");

      // Save session string if it changed
      const newSessionString = this.client!.session.save() as string | undefined;
      if (newSessionString && newSessionString !== this.sessionString) {
        this.sessionString = newSessionString;
        this.saveSession(newSessionString);
        console.log("Session updated and saved");
      }
    } catch (error) {
      console.error("Failed to connect to Telegram:", error);
      throw error;
    }
  }

  /**
   * Start authentication/login process
   */
  async start(options: LoginOptions): Promise<void> {
    if (!this.client) {
      throw new Error(
        "MTProto client not initialized. Call initialize() first.",
      );
    }

    try {
      await this.client.start(options);

      // Save session after successful login
      const newSessionString = this.client.session.save() as string | undefined;
      if (newSessionString && newSessionString !== this.sessionString) {
        this.sessionString = newSessionString;
        this.saveSession(newSessionString);
        console.log("Authentication successful, session saved");
      }
    } catch (error) {
      console.error("Authentication failed:", error);
      throw error;
    }
  }

  /**
   * Sign in using QR code — Enhanced with edge case handling.
   *
   * Handles:
   * - Token expiration + automatic re-generation (via library loop)
   * - DC migration (LoginTokenMigrateTo) — handled internally by GramJS
   * - 2FA (SESSION_PASSWORD_NEEDED) — routed to passwordCallback
   * - Server time sync for accurate expiration
   * - Race condition guard (isScanningComplete) to prevent double-processing
   *
   * @param qrCodeCallback Called each time a new QR token is generated
   * @param passwordCallback Called if 2FA password is needed after QR scan
   * @param onError Called on auth errors; return true to stop, false to continue
   * @param onStatus Optional status callback for UI updates (DC migration, 2FA, etc.)
   */
  async signInWithQrCode(
    qrCodeCallback: (qrCode: { token: Buffer; expires: number }) => void,
    passwordCallback?: (hint?: string) => Promise<string>,
    onError?: (err: Error) => Promise<boolean> | void,
    onStatus?: (status: QrLoginStatus) => void,
  ): Promise<unknown> {
    console.log("signInWithQrCode");
    if (!this.client) {
      throw new Error(
        "MTProto client not initialized. Call initialize() first.",
      );
    }

    // Reset race condition guard
    this.isScanningComplete = false;

    try {
      const user = await this.client.signInUserWithQrCode(
        {
          apiId: this.apiId,
          apiHash: this.apiHash,
        },
        {
          qrCode: async (qrCode) => {
            // Guard: don't process new tokens after scanning is complete
            if (this.isScanningComplete) return;

            // Use server time for more accurate expiration calculation.
            // The library provides `expires` as a Unix timestamp from Telegram's
            // server clock. We calculate our local approximation of server time
            // so the UI can display an accurate countdown.
            const serverTimeApprox = Math.floor(Date.now() / 1000);

            qrCodeCallback(qrCode);
            onStatus?.({
              type: "token",
              token: qrCode.token,
              expires: qrCode.expires,
              serverTime: serverTimeApprox,
            });
          },
          password: passwordCallback
            ? async (hint?: string) => {
                // 2FA flow triggered after successful QR scan
                this.isScanningComplete = true;
                onStatus?.({ type: "2fa_required", hint });
                return passwordCallback(hint);
              }
            : undefined,
          onError: async (err: Error): Promise<boolean> => {
            const errorMessage = err.message || "";

            // DC migration — the library handles this internally but we
            // notify the UI for status display
            if (errorMessage.includes("NETWORK_MIGRATE_")) {
              const dcMatch = errorMessage.match(/NETWORK_MIGRATE_(\d+)/);
              if (dcMatch) {
                onStatus?.({ type: "dc_migration", dcId: Number(dcMatch[1]) });
              }
              // Don't stop — let the library handle DC migration
              return false;
            }

            // 2FA / Session password — let password callback handle it
            if (
              errorMessage.includes("SESSION_PASSWORD_NEEDED") &&
              passwordCallback
            ) {
              this.isScanningComplete = true;
              onStatus?.({ type: "2fa_required" });
              if (onError) {
                const result = await onError(err);
                return result ?? false;
              }
              return false;
            }

            // Notify status listener
            onStatus?.({ type: "error", error: err });

            if (onError) {
              const result = await onError(err);
              return result ?? false;
            }
            console.error("QR code auth error:", err);
            return false;
          },
        },
      );

      // Mark scanning as complete
      this.isScanningComplete = true;
      onStatus?.({ type: "scanning_complete" });

      // Save session after successful login (critical after DC migration
      // where the session may have changed during the switchDC call)
      const newSessionString = this.client.session.save() as string | undefined;
      if (newSessionString && newSessionString !== this.sessionString) {
        this.sessionString = newSessionString;
        this.saveSession(newSessionString);
        console.log("QR code authentication successful, session saved");
      }

      onStatus?.({ type: "success" });
      return user;
    } catch (error) {
      this.isScanningComplete = true;

      const errorMessage =
        error instanceof Error ? error.message : String(error);

      if (errorMessage.includes("SESSION_PASSWORD_NEEDED")) {
        console.log(
          "SESSION_PASSWORD_NEEDED - password callback should handle this",
        );
      } else {
        console.error("QR code authentication failed:", error);
        onStatus?.({
          type: "error",
          error: error instanceof Error ? error : new Error(errorMessage),
        });
      }
      throw error;
    }
  }

  /**
   * Check if QR scanning is still in progress (not yet complete or errored).
   * Use this to avoid starting a new QR flow while one is active.
   */
  isQrScanningActive(): boolean {
    return !this.isScanningComplete;
  }

  /**
   * Get the Telegram client instance
   * @throws Error if client is not initialized
   */
  getClient(): TelegramClient {
    if (!this.client || !this.isInitialized) {
      throw new Error(
        "MTProto client not initialized. Call initialize() first.",
      );
    }
    return this.client;
  }

  /**
   * Check if the client is initialized
   */
  isReady(): boolean {
    return this.isInitialized && this.client !== undefined;
  }

  /**
   * Check if the client is connected
   */
  isClientConnected(): boolean {
    return this.isConnected && this.isReady();
  }

  /**
   * Get the current session string
   */
  getSessionString(): string {
    return this.sessionString;
  }

  /**
   * Check connection status and update user online status.
   * This calls getMe() which also updates the user's online status on Telegram.
   * Automatically initializes and connects if needed.
   * Concurrent calls await the same in-flight promise.
   */
  async checkConnection(userId?: string): Promise<boolean> {
    if (this.checkConnectionPromise) {
      return this.checkConnectionPromise;
    }

    this.checkConnectionPromise = this._doCheckConnection(userId).finally(() => {
      this.checkConnectionPromise = null;
    });
    return this.checkConnectionPromise;
  }

  private async _doCheckConnection(userId?: string): Promise<boolean> {
    try {
      if (!this.isInitialized || !this.client) {
        if (!userId) return false;
        await this.initialize(userId);
      }

      // Connect if not already connected
      if (!this.isConnected) {
        await this.connect();
      }

      // Check authorization
      const isAuthorized = await this.client!.checkAuthorization();
      if (!isAuthorized) {
        return false;
      }

      // Call getMe() to check connection and update online status with FLOOD_WAIT handling
      await this.handleFloodWait(async () => {
        await this.client!.getMe();
      });
      return true;
    } catch (error) {
      // Don't log FLOOD_WAIT as a warning - it's expected behavior
      if (error instanceof FloodWaitError) {
        console.debug(
          `Telegram connection check: FLOOD_WAIT ${error.seconds}s`,
        );
      } else {
        console.warn("Telegram connection check failed:", error);
      }
      return false;
    }
  }

  /**
   * Disconnect from Telegram
   */
  async disconnect(): Promise<void> {
    if (this.client && this.isConnected) {
      try {
        await this.client.disconnect();
        this.isConnected = false;
        console.log("Disconnected from Telegram");
      } catch (error) {
        console.error("Error disconnecting from Telegram:", error);
        throw error;
      }
    }
  }

  /**
   * Clear session from Redux, disconnect, and reset client state.
   * Use when the logged-in Telegram account does not match the app user (e.g. after QR connect).
   */
  async clearSessionAndDisconnect(userId?: string): Promise<void> {
    const uid = userId ?? this.userId;
    if (uid) {
      try {
        store.dispatch(setSessionString({ userId: uid, sessionString: null }));
      } catch (e) {
        console.warn("Failed to clear Telegram session from Redux:", e);
      }
    }
    await this.disconnect();
    this.client = undefined;
    this.isInitialized = false;
    this.isConnected = false;
    this.sessionString = "";
    this.userId = null;
    this.initializePromise = null;
    this.connectPromise = null;
    this.checkConnectionPromise = null;
  }

  /**
   * Send a message using the client with FLOOD_WAIT handling
   */
  async sendMessage(entity: string, message: string): Promise<void> {
    const client = this.getClient();
    if (!this.isClientConnected()) {
      await this.connect();
    }

    return this.handleFloodWait(async () => {
      await client.sendMessage(entity, { message });
    });
  }

  /**
   * Handle FLOOD_WAIT errors by waiting and retrying
   * @param operation The async operation to execute
   * @param maxRetries Maximum number of retry attempts (default: 3)
   * @param retryCount Current retry count (internal use)
   * @returns The result of the operation
   */
  private async handleFloodWait<T>(
    operation: () => Promise<T>,
    maxRetries = 3,
    retryCount = 0,
  ): Promise<T> {
    try {
      return await operation();
    } catch (error) {
      // Check if it's a FLOOD_WAIT error
      if (error instanceof FloodWaitError) {
        const waitSeconds = error.seconds;

        // If wait time is too long (more than 5 minutes), throw error
        if (waitSeconds > 300) {
          throw new Error(
            `FLOOD_WAIT: Too long wait time (${waitSeconds}s). Please try again later.`,
          );
        }

        // If we've exceeded max retries, throw error
        if (retryCount >= maxRetries) {
          throw new Error(
            `FLOOD_WAIT: Maximum retries exceeded. Wait ${waitSeconds}s before trying again.`,
          );
        }

        console.warn(
          `FLOOD_WAIT: Waiting ${waitSeconds} seconds before retry (attempt ${retryCount + 1}/${maxRetries})`,
        );

        // Wait for the specified time (convert to milliseconds)
        await new Promise((resolve) => setTimeout(resolve, waitSeconds * 1000));

        // Retry the operation
        return this.handleFloodWait(operation, maxRetries, retryCount + 1);
      }

      // If it's not a FLOOD_WAIT error, rethrow it
      throw error;
    }
  }

  /**
   * Execute an operation with FLOOD_WAIT error handling
   * This is a public utility method that can be used to wrap any Telegram API call
   * @param operation The async operation to execute
   * @param maxRetries Maximum number of retry attempts (default: 3)
   * @returns The result of the operation
   */
  async withFloodWaitHandling<T>(
    operation: () => Promise<T>,
    maxRetries = 3,
  ): Promise<T> {
    return this.handleFloodWait(operation, maxRetries);
  }

  /**
   * Invoke a raw Telegram API method with FLOOD_WAIT handling
   */
  async invoke<T = unknown>(
    request: Parameters<TelegramClient["invoke"]>[0],
  ): Promise<T> {
    const client = this.getClient();
    if (!this.isClientConnected()) {
      await this.connect();
    }

    return this.handleFloodWait(async () => {
      return client.invoke(request) as Promise<T>;
    });
  }

  private loadSession(): string | null {
    try {
      if (!this.userId) return null;
      const state = store.getState();
      const u = state.telegram.byUser[this.userId];
      return u?.sessionString ?? null;
    } catch (error) {
      console.error("Failed to load Telegram session from Redux:", error);
      return null;
    }
  }

  private saveSession(session: string): void {
    try {
      if (!this.userId) return;
      store.dispatch(
        setSessionString({ userId: this.userId, sessionString: session }),
      );
    } catch (error) {
      console.error("Failed to save Telegram session to Redux:", error);
    }
  }
}

export const mtprotoService = MTProtoService.getInstance();
export default mtprotoService;
