import { describe, expect, it } from "vitest";
import { detectReminderEvent, isAbnormalState } from "./reminder-state";
import type { BoardState, SessionRow } from "./session-state";

function makeRow(overrides: Partial<SessionRow> = {}): SessionRow {
  return {
    pid: 1234,
    name: "internal-name",
    title: "展示标题",
    sessionId: "session-1",
    cwd: "D:\\workspace\\pinglet",
    status: "working",
    statusUpdatedAt: 1000,
    alive: true,
    entrypoint: "claude",
    ...overrides,
  };
}

describe("isAbnormalState", () => {
  it("should treat blocked and interrupted as abnormal", () => {
    expect(isAbnormalState("blocked")).toBe(true);
    expect(isAbnormalState("interrupted")).toBe(true);
  });

  it("should treat working and done as normal", () => {
    expect(isAbnormalState("working")).toBe(false);
    expect(isAbnormalState("done")).toBe(false);
  });
});

describe("detectReminderEvent", () => {
  it("should only seed states when the first render already has blocked sessions", () => {
    const rows = [makeRow({ status: "waiting" })];

    const transition = detectReminderEvent(rows, new Map(), false);

    expect(transition.event).toBeNull();
    expect(transition.nextStates.get("pid:1234")).toBe("blocked");
  });

  it("should alert when a new abnormal session appears after the first render", () => {
    const rows = [makeRow({ pid: 42, status: "waiting" })];

    const transition = detectReminderEvent(rows, new Map(), true);

    expect(transition.event).toEqual({ kind: "alert", state: "blocked" });
  });

  it("should alert when a known session becomes interrupted", () => {
    const rows = [makeRow({ status: "failed", alive: true })];
    const previous = new Map<string, BoardState>([["pid:1234", "working"]]);

    const transition = detectReminderEvent(rows, previous, true);

    expect(transition.event).toEqual({ kind: "alert", state: "interrupted" });
  });

  it("should prefer blocked over interrupted when both appear in one poll", () => {
    const rows = [
      makeRow({ pid: 1, status: "failed", alive: true }),
      makeRow({ pid: 2, status: "waiting" }),
    ];
    const previous = new Map<string, BoardState>([
      ["pid:1", "working"],
      ["pid:2", "working"],
    ]);

    const transition = detectReminderEvent(rows, previous, true);

    expect(transition.event).toEqual({ kind: "alert", state: "blocked" });
  });

  it("should report completion when a working session becomes done", () => {
    const rows = [makeRow({ status: "idle" })];
    const previous = new Map<string, BoardState>([["pid:1234", "working"]]);

    const transition = detectReminderEvent(rows, previous, true);

    expect(transition.event).toEqual({ kind: "completion" });
  });

  it("should prefer the alert over completion in the same poll", () => {
    const rows = [makeRow({ pid: 1, status: "idle" }), makeRow({ pid: 2, status: "waiting" })];
    const previous = new Map<string, BoardState>([
      ["pid:1", "working"],
      ["pid:2", "working"],
    ]);

    const transition = detectReminderEvent(rows, previous, true);

    expect(transition.event).toEqual({ kind: "alert", state: "blocked" });
  });

  it("should emit no event when states are unchanged", () => {
    const rows = [makeRow({ status: "working" })];
    const previous = new Map<string, BoardState>([["pid:1234", "working"]]);

    const transition = detectReminderEvent(rows, previous, true);

    expect(transition.event).toBeNull();
  });

  it("should drop vanished sessions from the next state map", () => {
    const rows = [makeRow({ pid: 1, status: "working" })];
    const previous = new Map<string, BoardState>([
      ["pid:1", "working"],
      ["pid:2", "blocked"],
    ]);

    const transition = detectReminderEvent(rows, previous, true);

    expect([...transition.nextStates.keys()]).toEqual(["pid:1"]);
    expect(transition.event).toBeNull();
  });
});
