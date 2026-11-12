import { describe, expect, it } from "vitest";
import {
  DEFAULT_WINDOW_HEIGHT,
  DEFAULT_WINDOW_WIDTH,
  WINDOW_MAX_HEIGHT,
  WINDOW_MAX_WIDTH,
  WINDOW_MIN_HEIGHT,
  WINDOW_MIN_WIDTH,
  clampWindowHeight,
  clampWindowWidth,
  maxWindowHeight,
  normalizeStoredWindowSize,
} from "./window-size";

describe("defaults", () => {
  it("should keep the default size inside the allowed range", () => {
    expect(DEFAULT_WINDOW_WIDTH).toBeGreaterThanOrEqual(WINDOW_MIN_WIDTH);
    expect(DEFAULT_WINDOW_WIDTH).toBeLessThanOrEqual(WINDOW_MAX_WIDTH);
    expect(DEFAULT_WINDOW_HEIGHT).toBeGreaterThanOrEqual(WINDOW_MIN_HEIGHT);
    expect(DEFAULT_WINDOW_HEIGHT).toBeLessThanOrEqual(WINDOW_MAX_HEIGHT);
  });
});

describe("clampWindowWidth", () => {
  it("should round to the nearest integer", () => {
    expect(clampWindowWidth(320.6)).toBe(321);
  });

  it("should clamp below the minimum width", () => {
    expect(clampWindowWidth(100)).toBe(WINDOW_MIN_WIDTH);
  });

  it("should clamp above the maximum width", () => {
    expect(clampWindowWidth(1000)).toBe(WINDOW_MAX_WIDTH);
  });
});

describe("maxWindowHeight", () => {
  it("should clamp the screen height to the fixed maximum", () => {
    expect(maxWindowHeight(2000)).toBe(WINDOW_MAX_HEIGHT);
  });

  it("should use the screen height when it is smaller", () => {
    expect(maxWindowHeight(500.4)).toBe(500);
  });

  it("should never fall below the minimum height", () => {
    expect(maxWindowHeight(120)).toBe(WINDOW_MIN_HEIGHT);
  });
});

describe("clampWindowHeight", () => {
  it("should clamp to the provided maximum height", () => {
    expect(clampWindowHeight(900, 400)).toBe(400);
  });

  it("should clamp below the minimum height", () => {
    expect(clampWindowHeight(100, 400)).toBe(WINDOW_MIN_HEIGHT);
  });

  it("should round the height when it is in range", () => {
    expect(clampWindowHeight(333.6, 400)).toBe(334);
  });
});

describe("normalizeStoredWindowSize", () => {
  it("should return the size when the value has numeric width and height", () => {
    expect(normalizeStoredWindowSize({ width: 320, height: 480 })).toEqual({
      width: 320,
      height: 480,
    });
  });

  it("should return null when the value is not a plain object", () => {
    expect(normalizeStoredWindowSize(null)).toBeNull();
    expect(normalizeStoredWindowSize("320x480")).toBeNull();
    expect(normalizeStoredWindowSize([320, 480])).toBeNull();
  });

  it("should return null when width or height is missing or not a finite number", () => {
    expect(normalizeStoredWindowSize({ width: 320 })).toBeNull();
    expect(normalizeStoredWindowSize({ width: "320", height: 480 })).toBeNull();
    expect(normalizeStoredWindowSize({ width: 320, height: Number.NaN })).toBeNull();
    expect(normalizeStoredWindowSize({ width: Number.POSITIVE_INFINITY, height: 480 })).toBeNull();
  });
});
