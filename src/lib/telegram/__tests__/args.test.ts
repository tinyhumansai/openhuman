import { describe, it, expect } from "vitest";
import { optNumber, optString } from "../args";

describe("optNumber", () => {
  it("returns the number when args[key] is a valid number", () => {
    const args = { count: 42 };
    expect(optNumber(args, "count", 0)).toBe(42);
  });

  it("returns the number when args[key] is zero", () => {
    const args = { count: 0 };
    expect(optNumber(args, "count", 10)).toBe(0);
  });

  it("returns the number when args[key] is negative", () => {
    const args = { count: -5 };
    expect(optNumber(args, "count", 0)).toBe(-5);
  });

  it("returns fallback when args[key] is NaN", () => {
    const args = { count: NaN };
    expect(optNumber(args, "count", 100)).toBe(100);
  });

  it("returns fallback when args[key] is Infinity", () => {
    const args = { count: Infinity };
    expect(optNumber(args, "count", 50)).toBe(50);
  });

  it("returns fallback when args[key] is -Infinity", () => {
    const args = { count: -Infinity };
    expect(optNumber(args, "count", 50)).toBe(50);
  });

  it("returns fallback when args[key] is a string", () => {
    const args = { count: "123" };
    expect(optNumber(args, "count", 0)).toBe(0);
  });

  it("returns fallback when args[key] is undefined", () => {
    const args = {};
    expect(optNumber(args, "count", 25)).toBe(25);
  });

  it("returns fallback when args[key] is null", () => {
    const args = { count: null };
    expect(optNumber(args, "count", 99)).toBe(99);
  });

  it("returns fallback when args[key] is a boolean", () => {
    const args = { count: true };
    expect(optNumber(args, "count", 7)).toBe(7);
  });
});

describe("optString", () => {
  it("returns the string when args[key] is a valid string", () => {
    const args = { name: "alice" };
    expect(optString(args, "name")).toBe("alice");
  });

  it("returns empty string when args[key] is an empty string", () => {
    const args = { name: "" };
    expect(optString(args, "name")).toBe("");
  });

  it("returns undefined when args[key] is a number", () => {
    const args = { name: 123 };
    expect(optString(args, "name")).toBeUndefined();
  });

  it("returns undefined when args[key] is undefined", () => {
    const args = {};
    expect(optString(args, "name")).toBeUndefined();
  });

  it("returns undefined when args[key] is null", () => {
    const args = { name: null };
    expect(optString(args, "name")).toBeUndefined();
  });

  it("returns undefined when args[key] is a boolean", () => {
    const args = { name: false };
    expect(optString(args, "name")).toBeUndefined();
  });

  it("returns undefined when args[key] is an object", () => {
    const args = { name: { nested: "value" } };
    expect(optString(args, "name")).toBeUndefined();
  });
});
