/**
 * @ai-generated deepseek-v4-flash
 * 会话状态判定与展示文案：纯函数，不依赖 DOM。
 */

export type SessionRow = {
  pid: number | null;
  /** 内部会话名，仅用于激活终端窗口时匹配标题。 */
  name: string;
  /** 展示标题：用户标题、transcript 标题或目录名。 */
  title: string;
  sessionId: string | null;
  cwd: string | null;
  status: string;
  statusUpdatedAt: number | null;
  alive: boolean | null;
  entrypoint: string | null;
};

export type ListResult = {
  rows: SessionRow[];
  errors: string[];
};

/** 看板只保留四种状态，与 v1.2 设计一致。 */
export type BoardState = "blocked" | "interrupted" | "working" | "done";

export type SessionTooltip = {
  title: string;
  lines: string[];
  hint: string | null;
};

/** 双击会话行时可执行的动作；目前只支持 VSCode 与 CLI 两种入口。 */
export type SessionAction = "open-vscode" | "open-terminal";

export const BOARD_STATES: readonly BoardState[] = [
  "blocked",
  "interrupted",
  "working",
  "done",
];

type StateMeta = {
  label: string;
  priority: number;
  className: string;
};

const MILLISECONDS_PER_SECOND = 1000;
const SECONDS_PER_MINUTE = 60;
const MINUTES_PER_HOUR = 60;
const HOURS_PER_DAY = 24;

const STATUS_BUSY = "busy";
const STATUS_SHELL = "shell";
const STATUS_WAITING = "waiting";
const STATUS_IDLE = "idle";
const STATUS_WORKING = "working";
const STATUS_BLOCKED = "blocked";
const STATUS_DONE = "done";
const STATUS_FAILED = "failed";
const STATUS_STOPPED = "stopped";
const STATUS_CRASHED = "crashed";
const STATUS_STARTING = "starting";
const STATUS_RUNNING = "running";
const STATUS_RESUMING = "resuming";
const STATUS_ADOPTED = "adopted";

const PATH_SEPARATOR_PATTERN = /[\\/]+/;

/**
 * Claude Code 的不同版本会产出 4 个交互式状态（busy/shell/idle/waiting）
 * 与 agent view 的 working/blocked/done/failed/stopped 等状态，
 * 这里统一收敛到看板四态；未知状态按进程是否存活兜底。
 */
const STATUS_TO_STATE: Readonly<Record<string, BoardState>> = {
  [STATUS_WAITING]: "blocked",
  [STATUS_BLOCKED]: "blocked",
  [STATUS_BUSY]: "working",
  [STATUS_SHELL]: "working",
  [STATUS_WORKING]: "working",
  [STATUS_STARTING]: "working",
  [STATUS_RUNNING]: "working",
  [STATUS_RESUMING]: "working",
  [STATUS_ADOPTED]: "working",
  [STATUS_IDLE]: "done",
  [STATUS_DONE]: "done",
  [STATUS_FAILED]: "interrupted",
  [STATUS_CRASHED]: "interrupted",
  [STATUS_STOPPED]: "interrupted",
};

export const STATE_META: Record<BoardState, StateMeta> = {
  blocked: { label: "待处理", priority: 1, className: "state-blocked" },
  interrupted: {
    label: "已中断",
    priority: 2,
    className: "state-interrupted",
  },
  working: { label: "进行中", priority: 3, className: "state-working" },
  done: { label: "已完成", priority: 4, className: "state-done" },
};

export function classify(row: SessionRow): BoardState {
  // NOTE: 正常退出会删除会话文件；残留文件只可能来自异常退出或强制停止。
  // 只要进程已退出，无论最后状态是什么，都归为“已中断”。
  if (row.alive === false) {
    return "interrupted";
  }
  return STATUS_TO_STATE[row.status] ?? "working";
}

export function sortRows(rows: SessionRow[]): SessionRow[] {
  return [...rows].sort((left, right) => {
    const leftState = classify(left);
    const rightState = classify(right);
    const priorityDiff =
      STATE_META[leftState].priority - STATE_META[rightState].priority;
    if (priorityDiff !== 0) {
      return priorityDiff;
    }

    const isAscending = leftState === "blocked" || leftState === "working";
    if (isAscending) {
      const leftTime = left.statusUpdatedAt ?? Number.MAX_SAFE_INTEGER;
      const rightTime = right.statusUpdatedAt ?? Number.MAX_SAFE_INTEGER;
      return leftTime - rightTime;
    }

    const leftTime = left.statusUpdatedAt ?? 0;
    const rightTime = right.statusUpdatedAt ?? 0;
    return rightTime - leftTime;
  });
}

function formatDuration(timestamp: number | null, now: number): string {
  if (timestamp === null) {
    return "—";
  }

  const elapsedSeconds = Math.max(
    0,
    Math.floor((now - timestamp) / MILLISECONDS_PER_SECOND),
  );
  if (elapsedSeconds < SECONDS_PER_MINUTE) {
    return "刚刚";
  }

  const elapsedMinutes = Math.floor(elapsedSeconds / SECONDS_PER_MINUTE);
  if (elapsedMinutes < MINUTES_PER_HOUR) {
    return `${elapsedMinutes}m`;
  }

  const elapsedHours = Math.floor(elapsedMinutes / MINUTES_PER_HOUR);
  if (elapsedHours < HOURS_PER_DAY) {
    const remainingMinutes = elapsedMinutes % MINUTES_PER_HOUR;
    return remainingMinutes === 0
      ? `${elapsedHours}h`
      : `${elapsedHours}h ${remainingMinutes}m`;
  }

  return `${Math.floor(elapsedHours / HOURS_PER_DAY)}d`;
}

export function describeDuration(
  row: SessionRow,
  state: BoardState,
  now: number,
): string {
  const duration = formatDuration(row.statusUpdatedAt, now);

  switch (state) {
    case "blocked":
      return `等待 ${duration}`;
    case "interrupted":
      return `中断 ${duration}`;
    case "working":
      return row.status === STATUS_SHELL ? `命令 ${duration}` : `运行 ${duration}`;
    case "done":
      return `空闲 ${duration}`;
  }
}

/** 列表只显示当前文件夹名，完整路径由 tooltip 承载。 */
export function directoryName(path: string | null): string {
  if (path === null || path.trim() === "") {
    return "未知目录";
  }
  const segments = path.split(PATH_SEPARATOR_PATTERN).filter(Boolean);
  return segments[segments.length - 1] ?? path;
}

export function entrypointLabel(entrypoint: string | null): string | null {
  if (!entrypoint) {
    return null;
  }
  const normalized = entrypoint.toLowerCase();
  if (
    normalized.includes("vscode") ||
    normalized.includes("visual studio code")
  ) {
    return "vs-code";
  }
  if (normalized === "claude" || normalized.includes("cli")) {
    return "cli";
  }
  return entrypoint;
}

/** 只有 VSCode 与 CLI 入口支持双击跳转，其他入口一律不启动外部程序。 */
export function sessionAction(row: SessionRow): SessionAction | null {
  if (row.cwd === null) {
    return null;
  }
  const label = entrypointLabel(row.entrypoint);
  if (label === "vs-code") {
    return "open-vscode";
  }
  if (label === "cli") {
    return "open-terminal";
  }
  return null;
}

function formatTimestamp(timestamp: number | null): string {
  if (timestamp === null) {
    return "未知";
  }
  return new Date(timestamp).toLocaleString("zh-CN", { hour12: false });
}

export function buildTooltip(
  row: SessionRow,
  state: BoardState,
): SessionTooltip {
  const aliveLabel =
    row.alive === false ? "已退出" : row.alive === null ? "未知" : "运行中";
  const entrypoint = entrypointLabel(row.entrypoint);
  const lines = [
    `目录: ${row.cwd ?? "未知目录"}`,
    `状态: ${STATE_META[state].label}（${row.status}）`,
    `入口: ${entrypoint ?? "未知"}`,
    `进程: ${aliveLabel}`,
    `最后更新: ${formatTimestamp(row.statusUpdatedAt)}`,
    `PID: ${row.pid ?? "未知"}`,
  ];
  // 标题与内部名不同时补充内部名；同目录多会话可再靠会话 ID 区分
  if (row.name !== row.title) {
    lines.push(`内部名: ${row.name}`);
  }
  if (row.sessionId) {
    lines.push(`会话 ID: ${row.sessionId}`);
  }
  return {
    title: row.title,
    lines,
    hint: sessionAction(row) === null ? null : "双击激活会话窗口",
  };
}
