import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { relativeTime } from "./relativeTime";

describe("relativeTime", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns "just now" for under 1 minute ago', () => {
    vi.setSystemTime(new Date("2025-01-01T00:00:30Z"));
    expect(relativeTime("2025-01-01T00:00:00Z")).toBe("just now");
  });

  it("returns minutes for under 1 hour ago", () => {
    vi.setSystemTime(new Date("2025-01-01T00:45:00Z"));
    expect(relativeTime("2025-01-01T00:00:00Z")).toBe("45m ago");
  });

  it("returns hours for under 24 hours ago", () => {
    vi.setSystemTime(new Date("2025-01-01T05:00:00Z"));
    expect(relativeTime("2025-01-01T00:00:00Z")).toBe("5h ago");
  });

  it("returns days for 24+ hours ago", () => {
    vi.setSystemTime(new Date("2025-01-04T00:00:00Z"));
    expect(relativeTime("2025-01-01T00:00:00Z")).toBe("3d ago");
  });
});
