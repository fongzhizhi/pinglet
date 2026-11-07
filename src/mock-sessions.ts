/**
 * @ai-generated deepseek-v4-flash
 * 浏览器预览用的模拟会话数据，仅在 Vite dev 且非 Tauri 环境加载。
 */
import type { ListResult, SessionRow } from "./session-state";

const MILLISECONDS_PER_MINUTE = 60_000;
const MINUTES_PER_HOUR = 60;

/**
 * 生成覆盖看板四态的模拟数据：
 * 待处理（waiting）、已中断（进程已退出）、进行中（busy/shell/未知状态兜底）、已完成（idle）。
 * 时间戳相对当前时刻，避免时长文案随预览时间漂移。
 */
export function createMockResult(): ListResult {
  const now = Date.now();
  const minute = MILLISECONDS_PER_MINUTE;

  // 预览数据没有 transcript 标题，直接用会话名作为展示标题
  const rows: SessionRow[] = [
    {
      pid: 41001,
      name: "等待确认迁移方案",
      cwd: "C:\\Projects\\demo-alpha",
      status: "waiting",
      statusUpdatedAt: now - 2 * minute,
      alive: true,
      entrypoint: "claude-vscode",
    },
    {
      pid: 41002,
      name: "重构会话解析",
      cwd: "C:\\Projects\\demo-delta",
      status: "busy",
      statusUpdatedAt: now - 72 * minute,
      alive: true,
      entrypoint: "claude",
    },
    {
      pid: 41003,
      name: "运行单元测试",
      cwd: "C:\\Projects\\demo-gamma",
      status: "shell",
      statusUpdatedAt: now - 6 * minute,
      alive: true,
      entrypoint: "claude-vscode",
    },
    {
      pid: 41004,
      name: "整理变更日志",
      cwd: "C:\\Projects\\demo-epsilon",
      status: "idle",
      statusUpdatedAt: now - 3 * MINUTES_PER_HOUR * minute,
      alive: true,
      entrypoint: "claude-vscode",
    },
    {
      pid: 41005,
      name: "异常退出的旧会话",
      cwd: "C:\\Projects\\demo-zeta",
      status: "busy",
      statusUpdatedAt: now - 18 * minute,
      alive: false,
      entrypoint: "claude",
    },
    {
      pid: 41006,
      name: "被强制停止的旧会话",
      cwd: "C:\\Projects\\demo-eta",
      status: "idle",
      statusUpdatedAt: now - 26 * MINUTES_PER_HOUR * minute,
      alive: false,
      entrypoint: "claude",
    },
    {
      pid: 41007,
      name: "未知状态的会话",
      cwd: "C:\\Projects\\demo-theta",
      status: "paused",
      statusUpdatedAt: null,
      alive: true,
      entrypoint: null,
    },
  ].map((row) => ({ ...row, title: row.name, sessionId: null }));

  return { rows, errors: [] };
}
