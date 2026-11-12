import { describe, expect, it } from "vitest";
import { HEARTBEAT_OFFLINE_MS, HEARTBEAT_WARN_MS, resolveHeartbeatLevel } from "./heartbeat";

const NOW = 1_700_000_000_000;

describe("resolveHeartbeatLevel", () => {
  it("should report offline when no refresh has succeeded yet", () => {
    expect(resolveHeartbeatLevel(null, NOW)).toBe("offline");
  });

  it("should report ok when the last refresh is fresh", () => {
    expect(resolveHeartbeatLevel(NOW - 1000, NOW)).toBe("ok");
  });

  it("should report ok at the warning boundary", () => {
    expect(resolveHeartbeatLevel(NOW - HEARTBEAT_WARN_MS, NOW)).toBe("ok");
  });

  it("should report warn just past the warning threshold", () => {
    expect(resolveHeartbeatLevel(NOW - HEARTBEAT_WARN_MS - 1, NOW)).toBe("warn");
  });

  it("should report warn at the offline boundary", () => {
    expect(resolveHeartbeatLevel(NOW - HEARTBEAT_OFFLINE_MS, NOW)).toBe("warn");
  });

  it("should report offline just past the offline threshold", () => {
    expect(resolveHeartbeatLevel(NOW - HEARTBEAT_OFFLINE_MS - 1, NOW)).toBe("offline");
  });

  it("should report ok when the clock moves backwards", () => {
    expect(resolveHeartbeatLevel(NOW + 1000, NOW)).toBe("ok");
  });
});
