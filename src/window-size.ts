/**
 * @ai-generated deepseek-v4-flash
 * 窗口尺寸契约：尺寸上下限、屏幕可用高度裁剪与存档解析，不依赖 DOM 与 localStorage。
 */

export const WINDOW_MIN_WIDTH = 280;
export const WINDOW_MAX_WIDTH = 600;
export const WINDOW_MIN_HEIGHT = 200;
export const WINDOW_MAX_HEIGHT = 800;
// 默认宽度略宽于最小值，避免目录标签与标题过度截断；高度上限约 5 行会话，超出后列表内部滚动
export const DEFAULT_WINDOW_WIDTH = 320;
export const DEFAULT_WINDOW_HEIGHT = 320;

export type StoredWindowSize = {
  width: number;
  height: number;
};

export function clampWindowWidth(width: number): number {
  return Math.min(WINDOW_MAX_WIDTH, Math.max(WINDOW_MIN_WIDTH, Math.round(width)));
}

/** 高度上限同时受固定上限与屏幕可用高度约束，避免窗口超出屏幕。 */
export function maxWindowHeight(screenAvailHeight: number): number {
  return Math.max(WINDOW_MIN_HEIGHT, Math.min(WINDOW_MAX_HEIGHT, Math.round(screenAvailHeight)));
}

export function clampWindowHeight(height: number, maxHeight: number): number {
  return Math.min(maxHeight, Math.max(WINDOW_MIN_HEIGHT, Math.round(height)));
}

/** 解析 localStorage 中的尺寸存档；结构不符时返回 null，由调用方回落默认值。 */
export function normalizeStoredWindowSize(value: unknown): StoredWindowSize | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  if (
    typeof record.width !== "number" ||
    typeof record.height !== "number" ||
    !Number.isFinite(record.width) ||
    !Number.isFinite(record.height)
  ) {
    return null;
  }
  return { width: record.width, height: record.height };
}
