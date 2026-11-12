/**
 * @ai-generated deepseek-v4-flash
 * 心跳等级判定：根据距上次成功刷新的时长推导连接状态，不依赖 DOM。
 */

// 超过 5 秒未成功刷新视为警告，超过 15 秒视为离线
export const HEARTBEAT_WARN_MS = 5000;
export const HEARTBEAT_OFFLINE_MS = 15000;

export type HeartbeatLevel = "ok" | "warn" | "offline";

export function resolveHeartbeatLevel(
  lastSuccessfulRefreshAt: number | null,
  now: number,
): HeartbeatLevel {
  if (lastSuccessfulRefreshAt === null) {
    return "offline";
  }
  const age = now - lastSuccessfulRefreshAt;
  if (age > HEARTBEAT_OFFLINE_MS) {
    return "offline";
  }
  if (age > HEARTBEAT_WARN_MS) {
    return "warn";
  }
  return "ok";
}
