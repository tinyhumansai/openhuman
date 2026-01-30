/**
 * Telegram Update Manager
 *
 * Tracks pts/qts/seq state for the common update box and per-channel PTS.
 * Implements sorted queues for sequential update processing with gap detection
 * and automatic recovery via getDifference / getChannelDifference.
 *
 * Reference: Telegram-TT's src/api/gramjs/updates/updateManager.ts
 */

import { Api } from "telegram/tl";
import type { TelegramClient } from "telegram";
import bigInt from "big-integer";
import { mcpLog, mcpWarn } from "../lib/mcp/logger";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Common box state — tracks the global update sequence */
export interface CommonBoxState {
  seq: number;
  date: number;
  pts: number;
  qts: number;
}

/** A queued update with its sequencing metadata */
interface PtsUpdate {
  pts: number;
  ptsCount: number;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  update: any;
  channelId?: string;
}

interface SeqUpdate {
  seqStart: number;
  seqEnd: number;
  date: number;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  updates: any[];
}

export type UpdateHandler = (
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  update: any,
  source: "realtime" | "difference",
) => void;

// ---------------------------------------------------------------------------
// Sorted queue
// ---------------------------------------------------------------------------

class SortedQueue<T> {
  private items: T[] = [];

  constructor(private readonly compareFn: (a: T, b: T) => number) {}

  push(item: T): void {
    // Binary insert to maintain sort order
    let lo = 0;
    let hi = this.items.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (this.compareFn(this.items[mid], item) < 0) lo = mid + 1;
      else hi = mid;
    }
    this.items.splice(lo, 0, item);
  }

  peek(): T | undefined {
    return this.items[0];
  }

  shift(): T | undefined {
    return this.items.shift();
  }

  get length(): number {
    return this.items.length;
  }

  clear(): void {
    this.items = [];
  }
}

// ---------------------------------------------------------------------------
// Update Manager
// ---------------------------------------------------------------------------

const GAP_RECOVERY_DELAY_MS = 500;
const MAX_GAP_RECOVERY_ATTEMPTS = 3;

export class UpdateManager {
  private commonBoxState: CommonBoxState = { seq: 0, date: 0, pts: 0, qts: 0 };
  private channelPtsById: Record<string, number> = {};

  private seqQueue = new SortedQueue<SeqUpdate>(
    (a, b) => a.seqStart - b.seqStart,
  );
  private ptsQueues: Record<string, SortedQueue<PtsUpdate>> = {};

  private gapRecoveryTimer: ReturnType<typeof setTimeout> | null = null;
  private gapRecoveryAttempts = 0;
  private isRecovering = false;

  private client: TelegramClient | null = null;
  private updateHandler: UpdateHandler | null = null;

  // -------------------------------------------------------------------------
  // Initialization
  // -------------------------------------------------------------------------

  /**
   * Attach the MTProto client and an update handler callback.
   * Call once after the client is connected and authorized.
   */
  init(client: TelegramClient, handler: UpdateHandler): void {
    this.client = client;
    this.updateHandler = handler;
    mcpLog("UpdateManager initialized");
  }

  /**
   * Set the initial common box state (typically from the server after login).
   */
  setInitialState(state: Partial<CommonBoxState>): void {
    if (state.seq !== undefined) this.commonBoxState.seq = state.seq;
    if (state.date !== undefined) this.commonBoxState.date = state.date;
    if (state.pts !== undefined) this.commonBoxState.pts = state.pts;
    if (state.qts !== undefined) this.commonBoxState.qts = state.qts;
    mcpLog(
      `UpdateManager state set: seq=${this.commonBoxState.seq} pts=${this.commonBoxState.pts} qts=${this.commonBoxState.qts}`,
    );
  }

  /**
   * Set the initial PTS for a specific channel.
   */
  setChannelPts(channelId: string, pts: number): void {
    this.channelPtsById[channelId] = pts;
  }

  /**
   * Get current common box state (for persistence / diagnostics).
   */
  getCommonBoxState(): Readonly<CommonBoxState> {
    return { ...this.commonBoxState };
  }

  /**
   * Get current channel PTS map.
   */
  getChannelPtsMap(): Readonly<Record<string, number>> {
    return { ...this.channelPtsById };
  }

  // -------------------------------------------------------------------------
  // Update ingestion
  // -------------------------------------------------------------------------

  /**
   * Process an incoming update from the MTProto client.
   * Routes to the appropriate queue based on update type.
   */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  processUpdate(update: any): void {
    // Combined updates (Updates / UpdatesCombined) contain seq
    if (update instanceof Api.Updates || update instanceof Api.UpdatesCombined) {
      const seqStart =
        update instanceof Api.UpdatesCombined
          ? update.seqStart
          : update.seq;
      const seqEnd = update.seq;

      if (seqEnd > 0) {
        this.seqQueue.push({
          seqStart,
          seqEnd,
          date: update.date,
          updates: update.updates,
        });
        this.drainSeqQueue();
        return;
      }

      // seq=0 means these updates don't participate in sequencing
      for (const u of update.updates) {
        this.routePtsUpdate(u);
      }
      return;
    }

    // Short updates (UpdateShort, UpdateShortMessage, etc.)
    if (update instanceof Api.UpdateShort) {
      this.routePtsUpdate(update.update);
      return;
    }

    // Single update with pts
    if ("pts" in update && typeof update.pts === "number") {
      this.routePtsUpdate(update);
      return;
    }

    // No sequencing info — deliver directly
    this.deliver(update, "realtime");
  }

  // -------------------------------------------------------------------------
  // SEQ queue processing
  // -------------------------------------------------------------------------

  private drainSeqQueue(): void {
    while (this.seqQueue.length > 0) {
      const item = this.seqQueue.peek()!;

      if (item.seqStart === this.commonBoxState.seq + 1) {
        // Expected next update — apply it
        this.seqQueue.shift();
        this.commonBoxState.seq = item.seqEnd;
        this.commonBoxState.date = item.date;

        for (const u of item.updates) {
          this.routePtsUpdate(u);
        }
      } else if (item.seqStart <= this.commonBoxState.seq) {
        // Already applied (duplicate) — skip
        this.seqQueue.shift();
      } else {
        // Gap detected — schedule recovery
        mcpWarn(
          `SEQ gap: expected ${this.commonBoxState.seq + 1}, got ${item.seqStart}`,
        );
        this.scheduleGapRecovery();
        break;
      }
    }
  }

  // -------------------------------------------------------------------------
  // PTS queue processing
  // -------------------------------------------------------------------------

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private routePtsUpdate(update: any): void {
    if (!("pts" in update) || typeof update.pts !== "number") {
      this.deliver(update, "realtime");
      return;
    }

    const ptsCount =
      typeof update.ptsCount === "number" ? update.ptsCount : 0;

    // Determine if this is a channel-specific update
    const channelId = this.extractChannelId(update);

    if (channelId) {
      this.enqueuePtsUpdate(channelId, {
        pts: update.pts,
        ptsCount,
        update,
        channelId,
      });
    } else {
      this.enqueuePtsUpdate("__common__", {
        pts: update.pts,
        ptsCount,
        update,
      });
    }
  }

  private enqueuePtsUpdate(queueKey: string, item: PtsUpdate): void {
    if (!this.ptsQueues[queueKey]) {
      this.ptsQueues[queueKey] = new SortedQueue<PtsUpdate>(
        (a, b) => a.pts - b.pts,
      );
    }
    this.ptsQueues[queueKey].push(item);
    this.drainPtsQueue(queueKey);
  }

  private drainPtsQueue(queueKey: string): void {
    const queue = this.ptsQueues[queueKey];
    if (!queue) return;

    const localPts =
      queueKey === "__common__"
        ? this.commonBoxState.pts
        : (this.channelPtsById[queueKey] ?? 0);

    while (queue.length > 0) {
      const item = queue.peek()!;
      const expectedPts = localPts + item.ptsCount;

      if (item.pts === expectedPts || (localPts === 0 && item.ptsCount === 0)) {
        // Correct sequence — apply
        queue.shift();
        if (queueKey === "__common__") {
          this.commonBoxState.pts = item.pts;
        } else {
          this.channelPtsById[queueKey] = item.pts;
        }
        this.deliver(item.update, "realtime");
      } else if (item.pts <= localPts) {
        // Already applied (duplicate) — skip
        queue.shift();
      } else {
        // Gap detected
        mcpWarn(
          `PTS gap [${queueKey}]: local=${localPts}, got pts=${item.pts} ptsCount=${item.ptsCount}`,
        );
        this.scheduleGapRecovery(
          queueKey === "__common__" ? undefined : queueKey,
        );
        break;
      }
    }
  }

  // -------------------------------------------------------------------------
  // Gap recovery
  // -------------------------------------------------------------------------

  private scheduleGapRecovery(channelId?: string): void {
    if (this.isRecovering) return;
    if (this.gapRecoveryTimer) return;

    this.gapRecoveryTimer = setTimeout(() => {
      this.gapRecoveryTimer = null;
      void this.recoverGap(channelId);
    }, GAP_RECOVERY_DELAY_MS);
  }

  private async recoverGap(channelId?: string): Promise<void> {
    if (!this.client || this.isRecovering) return;

    if (this.gapRecoveryAttempts >= MAX_GAP_RECOVERY_ATTEMPTS) {
      mcpWarn(
        `Gap recovery: max attempts reached${channelId ? ` for channel ${channelId}` : ""}. Forcing full sync.`,
      );
      await this.forceSync(channelId);
      return;
    }

    this.isRecovering = true;
    this.gapRecoveryAttempts++;

    try {
      if (channelId) {
        await this.getChannelDifference(channelId);
      } else {
        await this.getDifference();
      }
      this.gapRecoveryAttempts = 0;
    } catch (error) {
      mcpWarn(`Gap recovery failed: ${error}`);
    } finally {
      this.isRecovering = false;
    }
  }

  /**
   * Fetch missing common box updates via updates.GetDifference.
   */
  private async getDifference(): Promise<void> {
    if (!this.client) return;

    mcpLog(
      `getDifference: pts=${this.commonBoxState.pts} date=${this.commonBoxState.date} qts=${this.commonBoxState.qts}`,
    );

    const result = await this.client.invoke(
      new Api.updates.GetDifference({
        pts: this.commonBoxState.pts,
        date: this.commonBoxState.date,
        qts: this.commonBoxState.qts,
      }),
    );

    if (result instanceof Api.updates.Difference) {
      this.applyDifference(result);
      this.commonBoxState.pts = result.state.pts;
      this.commonBoxState.seq = result.state.seq;
      this.commonBoxState.date = result.state.date;
      this.commonBoxState.qts = result.state.qts;
    } else if (result instanceof Api.updates.DifferenceSlice) {
      this.applyDifference(result);
      this.commonBoxState.pts = result.intermediateState.pts;
      this.commonBoxState.seq = result.intermediateState.seq;
      this.commonBoxState.date = result.intermediateState.date;
      this.commonBoxState.qts = result.intermediateState.qts;
      // More to fetch — recurse
      await this.getDifference();
    } else if (result instanceof Api.updates.DifferenceTooLong) {
      mcpWarn("DifferenceTooLong — forcing full sync");
      this.commonBoxState.pts = result.pts;
      await this.forceSync();
    }
    // DifferenceEmpty — nothing to do

    // Re-drain queues after applying difference
    this.drainSeqQueue();
    this.drainPtsQueue("__common__");
  }

  /**
   * Fetch missing channel-specific updates via updates.GetChannelDifference.
   */
  private async getChannelDifference(channelId: string): Promise<void> {
    if (!this.client) return;

    const pts = this.channelPtsById[channelId] ?? 0;
    if (pts === 0) {
      mcpWarn(`getChannelDifference: no pts for channel ${channelId}, skipping`);
      return;
    }

    mcpLog(`getChannelDifference: channel=${channelId} pts=${pts}`);

    try {
      const result = await this.client.invoke(
        new Api.updates.GetChannelDifference({
          channel: new Api.InputChannel({
            channelId: bigInt(channelId),
            accessHash: bigInt(0), // Will be resolved by the client layer
          }),
          pts,
          limit: 1000,
          filter: new Api.ChannelMessagesFilterEmpty(),
        }),
      );

      if (result instanceof Api.updates.ChannelDifference) {
        for (const msg of result.newMessages) {
          this.deliver(msg, "difference");
        }
        for (const update of result.otherUpdates) {
          this.deliver(update, "difference");
        }
        this.channelPtsById[channelId] = result.pts;

        if (!result.final) {
          // More to fetch
          await this.getChannelDifference(channelId);
        }
      } else if (result instanceof Api.updates.ChannelDifferenceTooLong) {
        mcpWarn(`ChannelDifferenceTooLong for ${channelId} — resetting`);
        if ("pts" in result && typeof result.pts === "number") {
          this.channelPtsById[channelId] = result.pts;
        }
        await this.forceSync(channelId);
      }
      // ChannelDifferenceEmpty — nothing to do

      this.drainPtsQueue(channelId);
    } catch (error) {
      mcpWarn(
        `getChannelDifference failed for ${channelId}: ${error}`,
      );
    }
  }

  /**
   * Force a full re-sync. Clears queues and notifies the handler.
   * The handler should re-fetch all chat data from scratch.
   */
  private async forceSync(channelId?: string): Promise<void> {
    if (channelId) {
      const queue = this.ptsQueues[channelId];
      if (queue) queue.clear();
    } else {
      this.seqQueue.clear();
      for (const key of Object.keys(this.ptsQueues)) {
        this.ptsQueues[key].clear();
      }
    }
    this.gapRecoveryAttempts = 0;

    // Notify handler about forced sync
    this.deliver(
      {
        _: "forceSync",
        channelId: channelId ?? null,
      },
      "difference",
    );
  }

  // -------------------------------------------------------------------------
  // Helpers
  // -------------------------------------------------------------------------

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private deliver(update: any, source: "realtime" | "difference"): void {
    if (this.updateHandler) {
      try {
        this.updateHandler(update, source);
      } catch (error) {
        console.error("[UpdateManager] Handler error:", error);
      }
    }
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private extractChannelId(update: any): string | undefined {
    // UpdateNewChannelMessage, UpdateEditChannelMessage, etc.
    if (update.message && update.message.peerId) {
      const peerId = update.message.peerId;
      if (peerId.channelId) return String(peerId.channelId);
    }
    if (update.channelId) return String(update.channelId);
    return undefined;
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private applyDifference(diff: { newMessages: any[]; otherUpdates: any[] }): void {
    for (const msg of diff.newMessages) {
      this.deliver(msg, "difference");
    }
    for (const update of diff.otherUpdates) {
      this.deliver(update, "difference");
    }
  }

  /**
   * Clean up timers and state.
   */
  destroy(): void {
    if (this.gapRecoveryTimer) {
      clearTimeout(this.gapRecoveryTimer);
      this.gapRecoveryTimer = null;
    }
    this.seqQueue.clear();
    for (const key of Object.keys(this.ptsQueues)) {
      this.ptsQueues[key].clear();
    }
    this.client = null;
    this.updateHandler = null;
    this.gapRecoveryAttempts = 0;
    this.isRecovering = false;
  }
}

/** Singleton update manager instance */
export const updateManager = new UpdateManager();
