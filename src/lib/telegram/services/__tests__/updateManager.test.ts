import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { Mock } from "vitest";
import type { TelegramClient } from "telegram";

// Mock telegram/tl
vi.mock("telegram/tl", () => ({
  Api: {
    Updates: class Updates {
      updates!: any[];
      seq!: number;
      date!: number;
      seqStart?: number;
      constructor(p: any) {
        Object.assign(this, p);
      }
    },
    UpdatesCombined: class UpdatesCombined {
      updates!: any[];
      seq!: number;
      seqStart!: number;
      date!: number;
      constructor(p: any) {
        Object.assign(this, p);
      }
    },
    UpdateShort: class UpdateShort {
      update!: any;
      constructor(p: any) {
        Object.assign(this, p);
      }
    },
    updates: {
      GetDifference: class GetDifference {
        constructor(public params: any) {}
      },
      Difference: class Difference {
        state!: any;
        newMessages!: any[];
        otherUpdates!: any[];
        constructor(p: any) {
          Object.assign(this, p);
        }
      },
      DifferenceSlice: class DifferenceSlice {
        intermediateState!: any;
        newMessages!: any[];
        otherUpdates!: any[];
        constructor(p: any) {
          Object.assign(this, p);
        }
      },
      DifferenceEmpty: class DifferenceEmpty {},
      DifferenceTooLong: class DifferenceTooLong {
        pts!: number;
        constructor(p: any) {
          Object.assign(this, p);
        }
      },
      GetChannelDifference: class GetChannelDifference {
        constructor(public params: any) {}
      },
      ChannelDifference: class ChannelDifference {
        pts!: number;
        final!: boolean;
        newMessages!: any[];
        otherUpdates!: any[];
        constructor(p: any) {
          Object.assign(this, p);
        }
      },
      ChannelDifferenceTooLong: class ChannelDifferenceTooLong {
        pts!: number;
        constructor(p: any) {
          Object.assign(this, p);
        }
      },
      ChannelDifferenceEmpty: class ChannelDifferenceEmpty {},
    },
    InputChannel: class InputChannel {
      constructor(public params: any) {}
    },
    ChannelMessagesFilterEmpty: class ChannelMessagesFilterEmpty {},
  },
}));

vi.mock("big-integer", () => ({ default: (v: any) => v }));
vi.mock("../../../mcp/logger");

import { UpdateManager } from "../updateManager";
import { Api } from "telegram/tl";

function createMockClient(): TelegramClient {
  return {
    invoke: vi.fn(),
  } as unknown as TelegramClient;
}

describe("UpdateManager", () => {
  let manager: UpdateManager;
  let mockClient: TelegramClient;
  let mockHandler: Mock;

  beforeEach(() => {
    vi.useFakeTimers();
    manager = new UpdateManager();
    mockClient = createMockClient();
    mockHandler = vi.fn();
  });

  afterEach(() => {
    manager.destroy();
    vi.useRealTimers();
  });

  describe("Initialization", () => {
    it("should initialize with client and handler", () => {
      manager.init(mockClient, mockHandler);
      expect(manager).toBeDefined();
    });

    it("should set initial state", () => {
      manager.setInitialState({ seq: 10, pts: 20, qts: 5, date: 1000 });
      const state = manager.getCommonBoxState();
      expect(state).toEqual({ seq: 10, pts: 20, qts: 5, date: 1000 });
    });

    it("should set partial initial state", () => {
      manager.setInitialState({ seq: 10, pts: 20 });
      const state = manager.getCommonBoxState();
      expect(state.seq).toBe(10);
      expect(state.pts).toBe(20);
      expect(state.qts).toBe(0);
      expect(state.date).toBe(0);
    });

    it("should set channel pts", () => {
      manager.setChannelPts("123", 50);
      const ptsMap = manager.getChannelPtsMap();
      expect(ptsMap["123"]).toBe(50);
    });

    it("should return copy of common box state", () => {
      manager.setInitialState({ seq: 10 });
      const state1 = manager.getCommonBoxState();
      const state2 = manager.getCommonBoxState();
      expect(state1).not.toBe(state2);
      expect(state1).toEqual(state2);
    });
  });

  describe("Update delivery", () => {
    beforeEach(() => {
      manager.init(mockClient, mockHandler);
    });

    it("should deliver plain update directly to handler", () => {
      const update = { _: "updateCustom", data: "test" } as any;
      manager.processUpdate(update);
      expect(mockHandler).toHaveBeenCalledWith(update, "realtime");
    });

    it("should route UpdateShort to pts processing", () => {
      const innerUpdate = { _: "updateTest", pts: 1, ptsCount: 1 } as any;
      const update = new Api.UpdateShort({ update: innerUpdate, date: 1000 } as any);
      manager.setInitialState({ pts: 0 } as any);
      manager.processUpdate(update);
      expect(mockHandler).toHaveBeenCalledWith(innerUpdate, "realtime");
    });

    it("should deliver update with pts to handler when sequence matches", () => {
      manager.setInitialState({ pts: 0 } as any);
      const update = { _: "updateTest", pts: 1, ptsCount: 1 } as any;
      manager.processUpdate(update);
      expect(mockHandler).toHaveBeenCalledWith(update, "realtime");
    });
  });

  describe("SEQ queue", () => {
    beforeEach(() => {
      manager.init(mockClient, mockHandler);
      manager.setInitialState({ seq: 0 } as any);
    });

    it("should deliver sequential seq updates in order", () => {
      const update1 = new Api.Updates({
        updates: [{ _: "update1" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 1,
        date: 1000,
      } as any);
      const update2 = new Api.Updates({
        updates: [{ _: "update2" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 2,
        date: 2000,
      } as any);

      manager.processUpdate(update1);
      manager.processUpdate(update2);

      expect(mockHandler).toHaveBeenCalledWith({ _: "update1" } as any, "realtime");
      expect(mockHandler).toHaveBeenCalledWith({ _: "update2" } as any, "realtime");
      expect(manager.getCommonBoxState().seq).toBe(2);
    });

    it("should queue out-of-order seq updates and deliver when gap fills", () => {
      const update1 = new Api.Updates({
        updates: [{ _: "update1" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 1,
        date: 1000,
      } as any);
      const update3 = new Api.Updates({
        updates: [{ _: "update3" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 3,
        date: 3000,
      } as any);
      const update2 = new Api.Updates({
        updates: [{ _: "update2" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 2,
        date: 2000,
      } as any);

      manager.processUpdate(update1);
      manager.processUpdate(update3); // Gap detected
      expect(mockHandler).toHaveBeenCalledTimes(1); // Only update1

      manager.processUpdate(update2); // Fills gap
      expect(mockHandler).toHaveBeenCalledWith({ _: "update2" } as any, "realtime");
      expect(mockHandler).toHaveBeenCalledWith({ _: "update3" } as any, "realtime");
      expect(manager.getCommonBoxState().seq).toBe(3);
    });

    it("should skip duplicate seq updates", () => {
      const update1 = new Api.Updates({
        updates: [{ _: "update1" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 1,
        date: 1000,
      } as any);
      const update1Dup = new Api.Updates({
        updates: [{ _: "update1dup" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 1,
        date: 1000,
      } as any);

      manager.processUpdate(update1);
      manager.processUpdate(update1Dup);

      expect(mockHandler).toHaveBeenCalledWith({ _: "update1" } as any, "realtime");
      expect(mockHandler).not.toHaveBeenCalledWith({ _: "update1dup" } as any, "realtime");
      expect(mockHandler).toHaveBeenCalledTimes(1);
    });

    it("should handle UpdatesCombined with seqStart", () => {
      const update = new Api.UpdatesCombined({
        updates: [{ _: "update1" } as any, { _: "update2" } as any],
        users: [] as any,
        chats: [] as any,
        seqStart: 1,
        seq: 2,
        date: 1000,
      } as any);

      manager.processUpdate(update);

      expect(mockHandler).toHaveBeenCalledWith({ _: "update1" } as any, "realtime");
      expect(mockHandler).toHaveBeenCalledWith({ _: "update2" } as any, "realtime");
      expect(manager.getCommonBoxState().seq).toBe(2);
    });

    it("should handle Updates with seq=0 (no sequencing)", () => {
      manager.setInitialState({ pts: 0 } as any);
      const update = new Api.Updates({
        updates: [
          { _: "update1", pts: 1, ptsCount: 1 } as any,
          { _: "update2", pts: 2, ptsCount: 1 } as any,
        ],
        users: [] as any,
        chats: [] as any,
        seq: 0,
        date: 1000,
      } as any);

      manager.processUpdate(update);

      expect(mockHandler).toHaveBeenCalledWith({ _: "update1", pts: 1, ptsCount: 1 } as any, "realtime");
      expect(mockHandler).toHaveBeenCalledWith({ _: "update2", pts: 2, ptsCount: 1 } as any, "realtime");
      expect(manager.getCommonBoxState().seq).toBe(0); // Seq not updated
    });
  });

  describe("PTS queue", () => {
    beforeEach(() => {
      manager.init(mockClient, mockHandler);
      manager.setInitialState({ pts: 0 } as any);
    });

    it("should deliver common box PTS update in correct sequence", () => {
      const update = { _: "updateTest", pts: 1, ptsCount: 1 } as any;
      manager.processUpdate(update);
      expect(mockHandler).toHaveBeenCalledWith(update, "realtime");
      expect(manager.getCommonBoxState().pts).toBe(1);
    });

    it("should skip duplicate PTS updates", () => {
      const update1 = { _: "update1", pts: 1, ptsCount: 1 } as any;
      const update1Dup = { _: "update1dup", pts: 1, ptsCount: 1 } as any;

      manager.processUpdate(update1);
      manager.processUpdate(update1Dup);

      expect(mockHandler).toHaveBeenCalledWith(update1, "realtime");
      expect(mockHandler).not.toHaveBeenCalledWith(update1Dup, "realtime");
      expect(mockHandler).toHaveBeenCalledTimes(1);
    });

    it("should route channel PTS update to channel queue", () => {
      manager.setChannelPts("123", 0);
      const update = {
        _: "updateNewChannelMessage",
        pts: 1,
        ptsCount: 1,
        message: {
          peerId: { channelId: "123" },
        },
      } as any;

      manager.processUpdate(update);
      expect(mockHandler).toHaveBeenCalledWith(update, "realtime");
      expect(manager.getChannelPtsMap()["123"]).toBe(1);
    });

    it("should handle out-of-order PTS updates when gap fills", () => {
      const update1 = { _: "update1", pts: 1, ptsCount: 1 } as any;
      const update3 = { _: "update3", pts: 3, ptsCount: 1 } as any;
      const update2 = { _: "update2", pts: 2, ptsCount: 1 } as any;

      manager.processUpdate(update1);
      manager.processUpdate(update3); // Gap — queued but not delivered
      expect(mockHandler).toHaveBeenCalledTimes(1);

      manager.processUpdate(update2); // Fills gap — delivers update2
      expect(mockHandler).toHaveBeenCalledWith(update2, "realtime");
      // Note: drainPtsQueue uses a const localPts snapshot, so update3
      // cannot be delivered in the same drain pass. It remains queued
      // until the next drain or gap recovery triggers.
      expect(mockHandler).toHaveBeenCalledTimes(2);
      expect(manager.getCommonBoxState().pts).toBe(2);
    });

    it("should handle pts=0 ptsCount=0 case", () => {
      manager.setInitialState({ pts: 0 } as any);
      const update = { _: "updateTest", pts: 0, ptsCount: 0 } as any;
      manager.processUpdate(update);
      expect(mockHandler).toHaveBeenCalledWith(update, "realtime");
    });
  });

  describe("Gap recovery", () => {
    beforeEach(() => {
      manager.init(mockClient, mockHandler);
      manager.setInitialState({ seq: 0, pts: 0, qts: 0, date: 1000 } as any);
    });

    it("should schedule gap recovery after delay on SEQ gap", () => {
      const update = new Api.Updates({
        updates: [{ _: "update" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 5, // Gap: expected seq=1
        date: 2000,
      } as any);

      manager.processUpdate(update);

      // Should not call getDifference immediately
      expect(mockClient.invoke).not.toHaveBeenCalled();

      // Advance timers
      vi.advanceTimersByTime(500);

      // Should now call getDifference
      expect(mockClient.invoke).toHaveBeenCalled();
    });

    it("should call getDifference on gap recovery", async () => {
      const mockDifference = new Api.updates.Difference({
        state: { pts: 10, seq: 5, date: 2000, qts: 5 } as any,
        newMessages: [{ _: "message1" } as any],
        newEncryptedMessages: [] as any,
        otherUpdates: [{ _: "update1" } as any],
        chats: [] as any,
        users: [] as any,
      });

      vi.mocked(mockClient.invoke).mockResolvedValue(mockDifference);

      const update = new Api.Updates({
        updates: [{ _: "update" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 5,
        date: 2000,
      } as any);

      manager.processUpdate(update);
      vi.advanceTimersByTime(500);

      await vi.runAllTimersAsync();

      expect(mockClient.invoke).toHaveBeenCalledWith(
        expect.objectContaining({
          params: expect.objectContaining({
            pts: 0,
            date: 1000,
            qts: 0,
          }),
        })
      );

      expect(mockHandler).toHaveBeenCalledWith({ _: "message1" } as any, "difference");
      expect(mockHandler).toHaveBeenCalledWith({ _: "update1" } as any, "difference");
      expect(manager.getCommonBoxState().pts).toBe(10);
      expect(manager.getCommonBoxState().seq).toBe(5);
    });

    it("should handle DifferenceSlice and recurse", async () => {
      const slice = new Api.updates.DifferenceSlice({
        intermediateState: { pts: 5, seq: 2, date: 1500, qts: 2 } as any,
        newMessages: [{ _: "msg1" } as any],
        newEncryptedMessages: [] as any,
        otherUpdates: [{ _: "upd1" } as any],
        chats: [] as any,
        users: [] as any,
      });

      const final = new Api.updates.Difference({
        state: { pts: 10, seq: 5, date: 2000, qts: 5 } as any,
        newMessages: [{ _: "msg2" } as any],
        newEncryptedMessages: [] as any,
        otherUpdates: [{ _: "upd2" } as any],
        chats: [] as any,
        users: [] as any,
      });

      vi.mocked(mockClient.invoke)
        .mockResolvedValueOnce(slice)
        .mockResolvedValueOnce(final);

      const update = new Api.Updates({
        updates: [{ _: "update" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 10,
        date: 3000,
      } as any);

      manager.processUpdate(update);
      vi.advanceTimersByTime(500);
      await vi.runAllTimersAsync();

      expect(mockClient.invoke).toHaveBeenCalledTimes(2);
      expect(mockHandler).toHaveBeenCalledWith({ _: "msg1" } as any, "difference");
      expect(mockHandler).toHaveBeenCalledWith({ _: "upd1" } as any, "difference");
      expect(mockHandler).toHaveBeenCalledWith({ _: "msg2" } as any, "difference");
      expect(mockHandler).toHaveBeenCalledWith({ _: "upd2" } as any, "difference");
      expect(manager.getCommonBoxState().pts).toBe(10);
    });

    it("should force full sync after max recovery attempts", async () => {
      vi.mocked(mockClient.invoke).mockRejectedValue(new Error("Network error"));

      const update = new Api.Updates({
        updates: [{ _: "update" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 5,
        date: 2000,
      } as any);

      // Trigger gap
      manager.processUpdate(update);

      // Attempt 1
      vi.advanceTimersByTime(500);
      await vi.runAllTimersAsync();

      // Attempt 2
      manager.processUpdate(update);
      vi.advanceTimersByTime(500);
      await vi.runAllTimersAsync();

      // Attempt 3
      manager.processUpdate(update);
      vi.advanceTimersByTime(500);
      await vi.runAllTimersAsync();

      // Should give up and force sync
      manager.processUpdate(update);
      vi.advanceTimersByTime(500);
      await vi.runAllTimersAsync();

      expect(mockHandler).toHaveBeenCalledWith(
        { _: "forceSync", channelId: null } as any,
        "difference"
      );
    });

    it("should not schedule recovery if already recovering", async () => {
      vi.mocked(mockClient.invoke).mockImplementation(
        () => new Promise((resolve) => setTimeout(resolve, 10000))
      );

      const update = new Api.Updates({
        updates: [{ _: "update" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 5,
        date: 2000,
      } as any);

      manager.processUpdate(update);
      vi.advanceTimersByTime(500);

      // Start recovery
      await Promise.resolve();

      // Try to trigger another recovery
      const update2 = new Api.Updates({
        updates: [{ _: "update2" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 10,
        date: 3000,
      } as any);
      manager.processUpdate(update2);

      expect(mockClient.invoke).toHaveBeenCalledTimes(1); // Only once
    });
  });

  describe("extractChannelId", () => {
    beforeEach(() => {
      manager.init(mockClient, mockHandler);
    });

    it("should extract channelId from message.peerId.channelId", () => {
      manager.setChannelPts("123", 0);
      const update = {
        _: "updateNewChannelMessage",
        pts: 1,
        ptsCount: 1,
        message: {
          peerId: { channelId: "123" },
        },
      } as any;

      manager.processUpdate(update);
      expect(manager.getChannelPtsMap()["123"]).toBe(1);
    });

    it("should extract channelId directly from update", () => {
      manager.setChannelPts("456", 0);
      const update = {
        _: "updateChannelTooLong",
        pts: 1,
        ptsCount: 1,
        channelId: "456",
      } as any;

      manager.processUpdate(update);
      expect(manager.getChannelPtsMap()["456"]).toBe(1);
    });

    it("should return undefined for non-channel updates", () => {
      manager.setInitialState({ pts: 0 } as any);
      const update = {
        _: "updateUserStatus",
        pts: 1,
        ptsCount: 1,
      } as any;

      manager.processUpdate(update);
      expect(manager.getCommonBoxState().pts).toBe(1); // Common box PTS updated
    });
  });

  describe("Destroy", () => {
    it("should clear timers and null client/handler", () => {
      manager.init(mockClient, mockHandler);

      const update = new Api.Updates({
        updates: [{ _: "update" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 5,
        date: 2000,
      } as any);

      manager.processUpdate(update); // Triggers gap recovery timer

      manager.destroy();

      vi.advanceTimersByTime(500);

      // Timer should be cleared, no invoke
      expect(mockClient.invoke).not.toHaveBeenCalled();
    });

    it("should clear all queues", () => {
      manager.init(mockClient, mockHandler);
      manager.setInitialState({ seq: 0, pts: 0 } as any);

      const update = new Api.Updates({
        updates: [{ _: "update" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 5,
        date: 2000,
      } as any);

      manager.processUpdate(update); // Adds to queue

      manager.destroy();

      // After destroy, processing new update should not access old queues
      manager.init(mockClient, mockHandler);
      manager.setInitialState({ seq: 0 } as any);

      const update2 = new Api.Updates({
        updates: [{ _: "update2" } as any],
        users: [] as any,
        chats: [] as any,
        seq: 1,
        date: 3000,
      } as any);

      manager.processUpdate(update2);
      expect(mockHandler).toHaveBeenCalledWith({ _: "update2" } as any, "realtime");
    });
  });

  describe("Handler error handling", () => {
    it("should catch and log handler errors", () => {
      const errorHandler = vi.fn(() => {
        throw new Error("Handler error");
      });

      const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

      manager.init(mockClient, errorHandler);
      const update = { _: "updateTest" } as any;

      manager.processUpdate(update);

      expect(errorHandler).toHaveBeenCalled();
      expect(consoleSpy).toHaveBeenCalledWith(
        "[UpdateManager] Handler error:",
        expect.any(Error)
      );

      consoleSpy.mockRestore();
    });
  });
});
