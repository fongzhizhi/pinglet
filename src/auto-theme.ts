/**
 * @ai-generated deepseek-v4-flash
 * 自动主题：采样窗口下方背景亮度，连续两次同向判定后切换深浅色。
 * 纯判定函数与采样控制器分离，阈值逻辑由 Vitest 覆盖。
 */
import { getEffectiveTheme, getSettings, setEffectiveTheme, type EffectiveTheme } from "./settings";
import { sampleBackdropLuminance } from "./system";

const SAMPLE_INTERVAL_MS = 10_000;
const SAMPLE_SETTLE_MS = 400;
const MIN_SWITCH_INTERVAL_MS = 10_000;
const REQUIRED_CONSISTENT_SAMPLES = 2;
/** 亮度高于该值时背景偏亮，改用暗色主题。 */
export const AUTO_THEME_DARK_THRESHOLD = 0.65;
/** 亮度低于该值时背景偏暗，改用亮色主题；中间区域保持当前主题。 */
export const AUTO_THEME_LIGHT_THRESHOLD = 0.35;

let sampleTimer: number | null = null;
let settleTimer: number | null = null;
let lastSwitchAt = 0;
let pendingCandidate: EffectiveTheme | null = null;
let pendingCount = 0;

/** 根据背景亮度给出应生效的主题；灰区保持现状，避免在阈值附近来回跳变。 */
export function resolveAutoTheme(luminance: number, current: EffectiveTheme): EffectiveTheme {
  if (luminance >= AUTO_THEME_DARK_THRESHOLD) {
    return "dark";
  }
  if (luminance <= AUTO_THEME_LIGHT_THRESHOLD) {
    return "light";
  }
  return current;
}

function isAutoThemeEnabled(): boolean {
  return getSettings().theme === "auto";
}

/** 设置切换到「自动」时启动周期采样；其他模式直接停止。 */
export function startAutoTheme(): void {
  stopAutoTheme();
  if (!isAutoThemeEnabled()) {
    return;
  }
  // 用户明确选择「自动」时立即给出结果，不等连续两次周期采样
  void sampleAutoTheme(true);
  sampleTimer = window.setInterval(() => {
    void sampleAutoTheme(false);
  }, SAMPLE_INTERVAL_MS);
}

export function stopAutoTheme(): void {
  if (sampleTimer !== null) {
    window.clearInterval(sampleTimer);
    sampleTimer = null;
  }
  if (settleTimer !== null) {
    window.clearTimeout(settleTimer);
    settleTimer = null;
  }
  pendingCandidate = null;
  pendingCount = 0;
}

/**
 * 窗口移动或重新可见后延迟采样；拖动期间只保留最后一次。
 * NOTE: 移动结束后背景已经确定，直接应用判定结果；否则要等两个周期采样（最长 20 秒）才切换。
 */
export function scheduleAutoThemeSample(): void {
  if (!isAutoThemeEnabled()) {
    return;
  }
  if (settleTimer !== null) {
    window.clearTimeout(settleTimer);
  }
  settleTimer = window.setTimeout(() => {
    settleTimer = null;
    void sampleAutoTheme(true);
  }, SAMPLE_SETTLE_MS);
}

/**
 * 采样一次背景亮度。
 * @param immediate 为 true 时不要求连续两次同向、也不受最小切换间隔限制，适用于移动结束 / 重新可见 / 刚开启自动主题。
 */
async function sampleAutoTheme(immediate: boolean): Promise<void> {
  if (!isAutoThemeEnabled()) {
    return;
  }

  let luminance: number | null;
  try {
    luminance = await sampleBackdropLuminance();
  } catch (error) {
    console.warn("pinglet: 自动主题采样失败", error);
    return;
  }
  if (luminance === null) {
    return;
  }
  // 采样是异步的，期间用户可能已切回固定主题
  if (!isAutoThemeEnabled()) {
    return;
  }

  const current = getEffectiveTheme();
  const candidate = resolveAutoTheme(luminance, current);
  if (candidate === current) {
    pendingCandidate = null;
    pendingCount = 0;
    return;
  }

  if (immediate) {
    pendingCandidate = null;
    pendingCount = 0;
    lastSwitchAt = Date.now();
    setEffectiveTheme(candidate);
    return;
  }

  if (candidate === pendingCandidate) {
    pendingCount += 1;
  } else {
    pendingCandidate = candidate;
    pendingCount = 1;
  }
  if (pendingCount < REQUIRED_CONSISTENT_SAMPLES) {
    return;
  }

  pendingCandidate = null;
  pendingCount = 0;
  const now = Date.now();
  if (now - lastSwitchAt < MIN_SWITCH_INTERVAL_MS) {
    return;
  }
  lastSwitchAt = now;
  setEffectiveTheme(candidate);
}
