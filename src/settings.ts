/**
 * @ai-generated deepseek-v4-flash
 * 设置单例与持久化：在 settings-schema 的纯逻辑之上提供订阅、localStorage 与 DOM 应用。
 * 全部设置只写入本机 WebView 的 localStorage，不触碰 Claude 配置。
 */
import {
  STORAGE_KEY,
  createDefaultSettings,
  normalizePanelTransparency,
  normalizeShortcut,
  parseSettings,
  type EffectiveTheme,
  type FontSize,
  type PanelEffectKey,
  type PanelEffects,
  type ReminderKey,
  type Settings,
  type ThemeMode,
} from "./settings-schema";

export {
  PANEL_TRANSPARENCY_MAX,
  PANEL_TRANSPARENCY_MIN,
  PANEL_TRANSPARENCY_STEP,
  SHORTCUT_MAX_LENGTH,
  createDefaultSettings,
  normalizePanelTransparency,
  normalizeShortcut,
  parseSettings,
  type EffectiveTheme,
  type FontSize,
  type PanelEffectKey,
  type PanelEffects,
  type ReminderKey,
  type Reminders,
  type Settings,
  type ThemeMode,
} from "./settings-schema";

/** 自动主题的生效结果单独持久化：通知弹窗是独立 WebView，不运行采样，需要复用主窗口的判定。 */
const EFFECTIVE_THEME_STORAGE_KEY = "pinglet.effective-theme.v1";
/** 主题切换时短暂挂到根节点，CSS 只在存在该类时开启颜色过渡。 */
const THEME_TRANSITION_CLASS = "is-theme-transitioning";
/** NOTE: 需与 styles.css 的 --theme-transition-duration（600ms）保持同步，额外留 40ms 收尾。 */
const THEME_TRANSITION_SETTLE_MS = 640;

const listeners = new Set<() => void>();
let settings = loadSettings();
// 自动主题需要异步采样背景；先沿用上次生效的深浅色，采样成功后再由 auto-theme 控制器覆盖
let effectiveTheme: EffectiveTheme =
  settings.theme === "auto" ? (readStoredEffectiveTheme() ?? "dark") : settings.theme;
// 记录已应用到 DOM 的主题：只有真正变化才播放过渡，避免每次设置变更都触发
let appliedTheme: EffectiveTheme | null = null;
// 默认关闭，主窗口初始化完成后延迟开启；通知弹窗等独立页面不开启
let isThemeTransitionEnabled = false;
let themeTransitionTimer: number | undefined;

export function getSettings(): Settings {
  return {
    ...settings,
    reminders: { ...settings.reminders },
    panelEffects: { ...settings.panelEffects },
  };
}

export function getEffectiveTheme(): EffectiveTheme {
  return effectiveTheme;
}

/**
 * 通知弹窗显示前重新读取主窗口持久化的设置与生效主题。
 * NOTE: 通知页是独立 WebView，settings 单例只在页面加载时初始化一次；
 * 主窗口后续修改主题、字号时不会自动同步过来。
 */
export function reloadSettingsFromStorage(): void {
  settings = loadSettings();
  effectiveTheme =
    settings.theme === "auto" ? (readStoredEffectiveTheme() ?? effectiveTheme) : settings.theme;
  notify();
}

/** 自动主题控制器采样后调用；只改 DOM，不写入用户主题模式。 */
export function setEffectiveTheme(theme: EffectiveTheme): void {
  if (effectiveTheme === theme) {
    return;
  }
  effectiveTheme = theme;
  applyTheme(theme);
}

export function subscribeSettings(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function setTheme(theme: ThemeMode): void {
  if (settings.theme === theme) {
    return;
  }
  settings.theme = theme;
  if (theme !== "auto") {
    effectiveTheme = theme;
  }
  persist();
  notify();
}

export function setFontSize(fontSize: FontSize): void {
  if (settings.fontSize === fontSize) {
    return;
  }
  settings.fontSize = fontSize;
  persist();
  notify();
}

export function setTransparencySetting(transparency: number): void {
  const normalized = normalizePanelTransparency(transparency);
  if (settings.transparency === normalized) {
    return;
  }
  settings.transparency = normalized;
  persist();
  notify();
}

export function setPanelEffect(key: PanelEffectKey, enabled: boolean): void {
  if (settings.panelEffects[key] === enabled) {
    return;
  }
  settings.panelEffects = { ...settings.panelEffects, [key]: enabled };
  persist();
  notify();
}

export function setReminder(key: ReminderKey, enabled: boolean): void {
  const reminders = { ...settings.reminders };

  if (key === "none") {
    if (enabled) {
      reminders.breathing = false;
      reminders.sound = false;
      reminders.shake = false;
      reminders.none = true;
    } else {
      reminders.none = false;
      reminders.breathing = true;
    }
  } else {
    reminders[key] = enabled;
    reminders.none = false;
    if (!reminders.breathing && !reminders.sound && !reminders.shake) {
      reminders.none = true;
    }
  }

  settings.reminders = reminders;
  persist();
  notify();
}

export function setCloseToTraySetting(enabled: boolean): void {
  if (settings.closeToTray === enabled) {
    return;
  }
  settings.closeToTray = enabled;
  persist();
  notify();
}

export function setClickThroughSetting(enabled: boolean): void {
  if (settings.clickThrough === enabled) {
    return;
  }
  settings.clickThrough = enabled;
  persist();
  notify();
}

export function setAutoStartSetting(enabled: boolean): void {
  if (settings.autoStart === enabled) {
    return;
  }
  settings.autoStart = enabled;
  persist();
  notify();
}

export function setShortcutSetting(shortcut: string): void {
  const normalized = normalizeShortcut(shortcut);
  if (settings.shortcut === normalized) {
    return;
  }
  settings.shortcut = normalized;
  persist();
  notify();
}

export function setPinnedSetting(pinned: boolean): void {
  if (settings.pinned === pinned) {
    return;
  }
  settings.pinned = pinned;
  persist();
  notify();
}

export function setDoNotDisturbSetting(enabled: boolean): void {
  if (settings.doNotDisturb === enabled) {
    return;
  }
  settings.doNotDisturb = enabled;
  persist();
  notify();
}

export function setSystemNotificationsSetting(enabled: boolean): void {
  if (settings.systemNotifications === enabled) {
    return;
  }
  settings.systemNotifications = enabled;
  persist();
  notify();
}

/** 一键恢复内置默认值，并返回新设置供调用方同步窗口与系统行为。 */
export function resetSettings(): Settings {
  settings = createDefaultSettings();
  effectiveTheme =
    settings.theme === "auto" ? (readStoredEffectiveTheme() ?? "dark") : settings.theme;
  persist();
  notify();
  return getSettings();
}

/**
 * 主题过渡开关：主窗口在首屏稳定后开启，避免启动时自动主题的首次修正被动画化。
 */
export function setThemeTransitionEnabled(enabled: boolean): void {
  isThemeTransitionEnabled = enabled;
  if (!enabled) {
    clearThemeTransition();
  }
}

export function applyTheme(theme: EffectiveTheme): void {
  const shouldTransition =
    appliedTheme !== null &&
    appliedTheme !== theme &&
    isThemeTransitionEnabled &&
    document.visibilityState === "visible" &&
    !window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  if (shouldTransition) {
    beginThemeTransition();
  }

  appliedTheme = theme;
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  writeStorage(EFFECTIVE_THEME_STORAGE_KEY, theme);
}

function beginThemeTransition(): void {
  const root = document.documentElement;
  root.classList.add(THEME_TRANSITION_CLASS);
  // NOTE: 强制一次样式计算，让过渡规则先以旧主题颜色为起点，再写入新 data-theme
  void root.offsetHeight;
  if (themeTransitionTimer !== undefined) {
    window.clearTimeout(themeTransitionTimer);
  }
  themeTransitionTimer = window.setTimeout(() => {
    themeTransitionTimer = undefined;
    root.classList.remove(THEME_TRANSITION_CLASS);
  }, THEME_TRANSITION_SETTLE_MS);
}

function clearThemeTransition(): void {
  if (themeTransitionTimer !== undefined) {
    window.clearTimeout(themeTransitionTimer);
    themeTransitionTimer = undefined;
  }
  document.documentElement.classList.remove(THEME_TRANSITION_CLASS);
}

export function applyFontSize(fontSize: FontSize): void {
  document.documentElement.dataset.fontSize = fontSize;
}

/** 透明度与 CSS 不透明度互为倒数：滑块越大，面板背景越透。 */
export function applyPanelTransparency(transparency: number): void {
  const alpha = Math.round((1 - transparency) * 100) / 100;
  document.documentElement.style.setProperty("--panel-opacity", String(alpha));
}

/** 面板行持续动效通过根节点 data 属性开关，CSS 负责具体动画。 */
function applyPanelEffects(effects: PanelEffects): void {
  document.documentElement.dataset.alertPulse = effects.alertPulse ? "on" : "off";
  document.documentElement.dataset.workingFlow = effects.workingFlow ? "on" : "off";
}

function readStorage(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch (error) {
    console.warn(`pinglet: 读取本地设置失败 (${key})`, error);
    return null;
  }
}

function writeStorage(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch (error) {
    console.warn(`pinglet: 写入本地设置失败 (${key})`, error);
  }
}

function readStoredEffectiveTheme(): EffectiveTheme | null {
  const stored = readStorage(EFFECTIVE_THEME_STORAGE_KEY);
  return stored === "dark" || stored === "light" ? stored : null;
}

function loadSettings(): Settings {
  return parseSettings(readStorage(STORAGE_KEY));
}

function persist(): void {
  writeStorage(STORAGE_KEY, JSON.stringify(settings));
}

function notify(): void {
  applyTheme(effectiveTheme);
  applyFontSize(settings.fontSize);
  applyPanelTransparency(settings.transparency);
  applyPanelEffects(settings.panelEffects);
  for (const listener of listeners) {
    try {
      listener();
    } catch (error) {
      console.error("pinglet: 设置变更监听器执行失败", error);
    }
  }
}

// 窗口隐藏时清理过渡类与计时器，避免恢复可见后残留的半程过渡
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "visible") {
    clearThemeTransition();
  }
});

applyTheme(effectiveTheme);
applyFontSize(settings.fontSize);
applyPanelTransparency(settings.transparency);
applyPanelEffects(settings.panelEffects);
