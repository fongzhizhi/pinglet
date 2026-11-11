import { describe, expect, it } from "vitest";
import {
  BOARD_STATES,
  STATE_META,
  applyHiddenSessions,
  buildTooltip,
  canHideSession,
  classify,
  createHiddenSessionSnapshot,
  describeDuration,
  directoryName,
  entrypointLabel,
  getRowKey,
  sessionAction,
  sortRows,
  type HiddenSessionSnapshot,
  type SessionRow,
} from "./session-state";

const NOW = Date.UTC(2026, 0, 1, 12, 0, 0);
const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

function makeRow(overrides: Partial<SessionRow> = {}): SessionRow {
  return {
    pid: 1234,
    name: "internal-name",
    title: "展示标题",
    sessionId: "session-1",
    cwd: "D:\\workspace\\pinglet",
    status: "working",
    statusUpdatedAt: NOW - MINUTE,
    alive: true,
    entrypoint: "claude",
    ...overrides,
  };
}

describe("classify", () => {
  it.each([
    ["waiting", "blocked"],
    ["blocked", "blocked"],
    ["busy", "working"],
    ["shell", "working"],
    ["working", "working"],
    ["starting", "working"],
    ["running", "working"],
    ["resuming", "working"],
    ["adopted", "working"],
    ["idle", "done"],
    ["done", "done"],
    ["failed", "interrupted"],
    ["crashed", "interrupted"],
    ["stopped", "interrupted"],
    ["unknown", "working"],
  ] as const)("should classify %s as %s when the process is alive", (status, expected) => {
    const row = makeRow({ status });

    const state = classify(row);

    expect(state).toBe(expected);
  });

  it("should classify a dead process as interrupted when status looks healthy", () => {
    const row = makeRow({ status: "working", alive: false });

    const state = classify(row);

    expect(state).toBe("interrupted");
  });

  it("should classify unknown liveness as alive when status is unknown", () => {
    const row = makeRow({ status: "unknown", alive: null });

    const state = classify(row);

    expect(state).toBe("working");
  });
});

describe("STATE_META", () => {
  it("should expose the four board states in priority order", () => {
    const states = [...BOARD_STATES];
    const priorities = states.map((state) => STATE_META[state].priority);

    expect(states).toEqual(["blocked", "interrupted", "working", "done"]);
    expect(priorities).toEqual([1, 2, 3, 4]);
  });
});

describe("sortRows", () => {
  it("should order blocked, interrupted, working and done when states differ", () => {
    const rows = [
      makeRow({ name: "done", status: "idle", statusUpdatedAt: NOW - 3 * MINUTE }),
      makeRow({ name: "working", status: "working", statusUpdatedAt: NOW - 2 * MINUTE }),
      makeRow({ name: "interrupted", status: "failed", statusUpdatedAt: NOW - 1 * MINUTE }),
      makeRow({ name: "blocked", status: "waiting", statusUpdatedAt: NOW - 4 * MINUTE }),
    ];

    const sorted = sortRows(rows);

    expect(sorted.map((row) => row.name)).toEqual(["blocked", "interrupted", "working", "done"]);
  });

  it("should sort blocked and working sessions oldest first and missing timestamps last", () => {
    const rows = [
      makeRow({ name: "newer", status: "waiting", statusUpdatedAt: NOW - MINUTE }),
      makeRow({ name: "missing", status: "waiting", statusUpdatedAt: null }),
      makeRow({ name: "older", status: "waiting", statusUpdatedAt: NOW - 10 * MINUTE }),
    ];

    const sorted = sortRows(rows);

    expect(sorted.map((row) => row.name)).toEqual(["older", "newer", "missing"]);
  });

  it("should sort interrupted and done sessions newest first and missing timestamps last", () => {
    const rows = [
      makeRow({ name: "older", status: "idle", statusUpdatedAt: NOW - 10 * MINUTE }),
      makeRow({ name: "missing", status: "idle", statusUpdatedAt: null }),
      makeRow({ name: "newer", status: "idle", statusUpdatedAt: NOW - MINUTE }),
    ];

    const sorted = sortRows(rows);

    expect(sorted.map((row) => row.name)).toEqual(["newer", "older", "missing"]);
  });

  it("should not mutate the input array", () => {
    const rows = [
      makeRow({ name: "a", status: "working" }),
      makeRow({ name: "b", status: "waiting" }),
    ];
    const snapshot = [...rows];

    sortRows(rows);

    expect(rows).toEqual(snapshot);
  });
});

describe("describeDuration", () => {
  it.each([
    [null, "等待 —"],
    [NOW - 30_000, "等待 刚刚"],
    [NOW - 5 * MINUTE, "等待 5m"],
    [NOW - (2 * HOUR + 3 * MINUTE), "等待 2h 3m"],
    [NOW - 2 * HOUR, "等待 2h"],
    [NOW - 3 * DAY, "等待 3d"],
  ] as const)("should format a timestamp as %s", (statusUpdatedAt, expected) => {
    const row = makeRow({ status: "waiting", statusUpdatedAt });

    const text = describeDuration(row, "blocked", NOW);

    expect(text).toBe(expected);
  });

  it("should use the status-specific prefix when state is working", () => {
    const workingRow = makeRow({ status: "working", statusUpdatedAt: NOW - 5 * MINUTE });
    const shellRow = makeRow({ status: "shell", statusUpdatedAt: NOW - 5 * MINUTE });
    const failedRow = makeRow({ status: "failed", statusUpdatedAt: NOW - 5 * MINUTE });
    const idleRow = makeRow({ status: "idle", statusUpdatedAt: NOW - 5 * MINUTE });

    expect(describeDuration(workingRow, "working", NOW)).toBe("运行 5m");
    expect(describeDuration(shellRow, "working", NOW)).toBe("命令 5m");
    expect(describeDuration(failedRow, "interrupted", NOW)).toBe("中断 5m");
    expect(describeDuration(idleRow, "done", NOW)).toBe("空闲 5m");
  });

  it("should clamp future timestamps to just now", () => {
    const row = makeRow({ status: "waiting", statusUpdatedAt: NOW + MINUTE });

    const text = describeDuration(row, "blocked", NOW);

    expect(text).toBe("等待 刚刚");
  });
});

describe("directoryName", () => {
  it.each([
    ["D:\\workspace\\pinglet", "pinglet"],
    ["/home/user/project", "project"],
    ["D:\\workspace\\pinglet\\", "pinglet"],
    ["relative", "relative"],
    [null, "未知目录"],
    ["", "未知目录"],
    ["   ", "未知目录"],
  ] as const)("should extract a directory name from %s", (path, expected) => {
    const name = directoryName(path);

    expect(name).toBe(expected);
  });
});

describe("entrypointLabel", () => {
  it.each([
    [null, null],
    ["", null],
    ["vscode", "vs-code"],
    ["Visual Studio Code", "vs-code"],
    ["claude-vscode", "vs-code"],
    ["claude", "cli"],
    ["claude-cli", "cli"],
    ["cli", "cli"],
    ["terminal", "terminal"],
  ] as const)("should label %s as %s", (entrypoint, expected) => {
    const label = entrypointLabel(entrypoint);

    expect(label).toBe(expected);
  });
});

describe("sessionAction", () => {
  it("should return null when the working directory is missing", () => {
    const row = makeRow({ cwd: null });

    const action = sessionAction(row);

    expect(action).toBeNull();
  });

  it("should map VSCode and CLI entrypoints to their actions", () => {
    const vscodeRow = makeRow({ entrypoint: "vscode" });
    const cliRow = makeRow({ entrypoint: "claude" });

    expect(sessionAction(vscodeRow)).toBe("open-vscode");
    expect(sessionAction(cliRow)).toBe("open-terminal");
  });

  it("should return null when the entrypoint is unsupported", () => {
    const row = makeRow({ entrypoint: "terminal" });

    const action = sessionAction(row);

    expect(action).toBeNull();
  });
});

describe("buildTooltip", () => {
  it("should include path, state, entrypoint, process, timestamp and pid", () => {
    const row = makeRow({ status: "waiting", alive: false, entrypoint: "vscode" });

    const tooltip = buildTooltip(row, "blocked");

    expect(tooltip.title).toBe("展示标题");
    expect(tooltip.lines).toContain("目录: D:\\workspace\\pinglet");
    expect(tooltip.lines).toContain("状态: 待处理（waiting）");
    expect(tooltip.lines).toContain("入口: vs-code");
    expect(tooltip.lines).toContain("进程: 已退出");
    expect(tooltip.lines.some((line) => line.startsWith("最后更新: "))).toBe(true);
    expect(tooltip.lines).toContain("PID: 1234");
    expect(tooltip.lines).toContain("内部名: internal-name");
    expect(tooltip.lines).toContain("会话 ID: session-1");
    expect(tooltip.hint).toBe("双击激活会话窗口");
  });

  it("should omit the internal name when it matches the title", () => {
    const row = makeRow({ name: "same", title: "same" });

    const tooltip = buildTooltip(row, "working");

    expect(tooltip.lines.some((line) => line.startsWith("内部名:"))).toBe(false);
  });

  it("should omit the session id when it is unavailable", () => {
    const row = makeRow({ sessionId: null });

    const tooltip = buildTooltip(row, "working");

    expect(tooltip.lines.some((line) => line.startsWith("会话 ID:"))).toBe(false);
  });

  it("should fall back for missing values and unsupported actions", () => {
    const row = makeRow({
      pid: null,
      cwd: null,
      sessionId: null,
      status: "idle",
      statusUpdatedAt: null,
      alive: null,
      entrypoint: null,
    });

    const tooltip = buildTooltip(row, "done");

    expect(tooltip.lines).toContain("目录: 未知目录");
    expect(tooltip.lines).toContain("状态: 已完成（idle）");
    expect(tooltip.lines).toContain("入口: 未知");
    expect(tooltip.lines).toContain("进程: 未知");
    expect(tooltip.lines).toContain("最后更新: 未知");
    expect(tooltip.lines).toContain("PID: 未知");
    expect(tooltip.hint).toBeNull();
  });
});

describe("getRowKey", () => {
  it("should prefer pid, then cwd, then name when building the identity", () => {
    const withPid = makeRow({ pid: 42, cwd: "D:\\a", name: "n" });
    const withCwd = makeRow({ pid: null, cwd: "D:\\a", name: "n" });
    const withName = makeRow({ pid: null, cwd: null, name: "n" });

    expect(getRowKey(withPid)).toBe("pid:42");
    expect(getRowKey(withCwd)).toBe("cwd:D:\\a");
    expect(getRowKey(withName)).toBe("name:n");
  });
});

describe("canHideSession", () => {
  it.each([
    [false, "blocked"],
    [false, "interrupted"],
    [false, "working"],
    [true, "done"],
  ] as const)("should return %s when the state is %s", (expected, state) => {
    expect(canHideSession(state)).toBe(expected);
  });
});

describe("applyHiddenSessions", () => {
  it("should remove a hidden session from visible rows when its snapshot is unchanged", () => {
    const row = makeRow({ status: "idle", statusUpdatedAt: NOW - MINUTE });
    const hidden = new Map<string, HiddenSessionSnapshot>([
      [getRowKey(row), createHiddenSessionSnapshot(row)],
    ]);

    const visible = applyHiddenSessions([row], hidden);

    expect(visible).toEqual([]);
    expect(hidden.size).toBe(1);
  });

  it("should keep other sessions visible when one matching session is hidden", () => {
    const hiddenRow = makeRow({ pid: 1, status: "idle" });
    const visibleRow = makeRow({ pid: 2, status: "working" });
    const hidden = new Map<string, HiddenSessionSnapshot>([
      [getRowKey(hiddenRow), createHiddenSessionSnapshot(hiddenRow)],
    ]);

    const visible = applyHiddenSessions([hiddenRow, visibleRow], hidden);

    expect(visible.map((row) => row.pid)).toEqual([2]);
  });

  it("should restore the session and drop the snapshot when the row disappears", () => {
    const row = makeRow({ status: "idle" });
    const hidden = new Map<string, HiddenSessionSnapshot>([
      [getRowKey(row), createHiddenSessionSnapshot(row)],
    ]);

    const visible = applyHiddenSessions([], hidden);

    expect(visible).toEqual([]);
    expect(hidden.size).toBe(0);
  });

  it("should restore the session when its status changes", () => {
    const row = makeRow({ status: "idle" });
    const hidden = new Map<string, HiddenSessionSnapshot>([
      [getRowKey(row), createHiddenSessionSnapshot(row)],
    ]);
    const changed = { ...row, status: "working" };

    const visible = applyHiddenSessions([changed], hidden);

    expect(visible).toEqual([changed]);
    expect(hidden.size).toBe(0);
  });

  it("should restore the session when statusUpdatedAt changes after re-entering done", () => {
    const row = makeRow({ status: "idle", statusUpdatedAt: NOW - MINUTE });
    const hidden = new Map<string, HiddenSessionSnapshot>([
      [getRowKey(row), createHiddenSessionSnapshot(row)],
    ]);
    const changed = { ...row, statusUpdatedAt: NOW };

    const visible = applyHiddenSessions([changed], hidden);

    expect(visible).toEqual([changed]);
    expect(hidden.size).toBe(0);
  });

  it("should restore the session when the process exits while hidden", () => {
    const row = makeRow({ status: "idle", alive: true });
    const hidden = new Map<string, HiddenSessionSnapshot>([
      [getRowKey(row), createHiddenSessionSnapshot(row)],
    ]);
    const changed = { ...row, alive: false };

    const visible = applyHiddenSessions([changed], hidden);

    expect(visible).toEqual([changed]);
    expect(hidden.size).toBe(0);
  });

  it("should restore the session when the pid is reused by another session", () => {
    const row = makeRow({ status: "idle", sessionId: "session-1" });
    const hidden = new Map<string, HiddenSessionSnapshot>([
      [getRowKey(row), createHiddenSessionSnapshot(row)],
    ]);
    const changed = { ...row, sessionId: "session-2" };

    const visible = applyHiddenSessions([changed], hidden);

    expect(visible).toEqual([changed]);
    expect(hidden.size).toBe(0);
  });
});
