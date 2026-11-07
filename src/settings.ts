/**
 * @ai-generated deepseek-v4-flash
 * 设置模型与持久化：暗色/亮色主题、字号、面板透明度、提醒模式、关闭行为、鼠标穿透、开机自启、快捷键与窗口固定状态。
 * 全部设置只写入本机 WebView 的 localStorage，不触碰 Claude 配置。
 */

export type ThemeMode = "dark" | "light";

export type FontSize = "small" | "standard" | "large";

export type ReminderKey = "breathing" | "sound" | "shake" | "none";

export type Reminders = Record<ReminderKey, boolean>;

export type Settings = {
  theme: ThemeMode;
  fontSize: FontSize;
  /** 面板背景透明度：0–0.7，0 为完全不透明 */
  transparency: number;
  reminders: Reminders;
  closeToTray: boolean;
  clickThrough: boolean;
  autoStart: boolean;
  shortcut: string;
  pinned: boolean;
};

const STORAGE_KEY = "pinglet.settings.v1";
const DEFAULT_SHORTCUT = "Alt+Shift+P";
const SHORTCUT_MAX_LENGTH = 64;
const THEME_DARK: ThemeMode = "dark";
const FONT_SIZE_STANDARD: FontSize = "standard";
export const PANEL_TRANSPARENCY_MIN = 0;
export const PANEL_TRANSPARENCY_MAX = 0.7;
export const PANEL_TRANSPARENCY_STEP = 0.05;
const DEFAULT_PANEL_TRANSPARENCY = PANEL_TRANSPARENCY_MIN;
const FONT_SIZES: ReadonlySet<FontSize> = new Set(["small", "standard", "large"]);

const DEFAULT_REMINDERS: Reminders = {
  breathing: true,
  sound: true,
  shake: true,
  none: false,
};

function createDefaultSettings(): Settings {
  return {
    theme: THEME_DARK,
    fontSize: FONT_SIZE_STANDARD,
    transparency: DEFAULT_PANEL_TRANSPARENCY,
    reminders: { ...DEFAULT_REMINDERS },
    closeToTray: false,
    clickThrough: false,
    autoStart: false,
    shortcut: DEFAULT_SHORTCUT,
    pinned: false,
  };
}

const listeners = new Set<() => void>();
let settings = loadSettings();

export function getSettings(): Settings {
  return {
    ...settings,
    reminders: { ...settings.reminders },
  };
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
  const normalized = shortcut.trim().slice(0, SHORTCUT_MAX_LENGTH);
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

/** 一键恢复内置默认值，并返回新设置供调用方同步窗口与系统行为。 */
export function resetSettings(): Settings {
  settings = createDefaultSettings();
  persist();
  notify();
  return getSettings();
}

export function applyTheme(theme: ThemeMode): void {
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
}

export function applyFontSize(fontSize: FontSize): void {
  document.documentElement.dataset.fontSize = fontSize;
}

/** 透明度与 CSS 不透明度互为倒数：滑块越大，面板背景越透。 */
export function applyPanelTransparency(transparency: number): void {
  const alpha = Math.round((1 - transparency) * 100) / 100;
  document.documentElement.style.setProperty("--panel-opacity", String(alpha));
}

/** 限制到滑块范围并吸附到 5% 步进，避免浮点误差影响比较与持久化。 */
function normalizePanelTransparency(transparency: number): number {
  const clamped = Math.min(PANEL_TRANSPARENCY_MAX, Math.max(PANEL_TRANSPARENCY_MIN, transparency));
  const stepped = Math.round(clamped / PANEL_TRANSPARENCY_STEP) * PANEL_TRANSPARENCY_STEP;
  return Math.round(stepped * 100) / 100;
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseReminders(value: unknown): Reminders {
  if (!isRecord(value)) {
    return { ...DEFAULT_REMINDERS };
  }

  const reminders: Reminders = {
    breathing: value.breathing === true,
    sound: value.sound === true,
    shake: value.shake === true,
    none: value.none === true,
  };
  if (reminders.none) {
    return { breathing: false, sound: false, shake: false, none: true };
  }
  if (!reminders.breathing && !reminders.sound && !reminders.shake) {
    return { ...DEFAULT_REMINDERS };
  }
  return reminders;
}

function parseSettings(raw: string | null): Settings {
  const fallback = createDefaultSettings();
  if (raw === null) {
    return fallback;
  }

  try {
    const value: unknown = JSON.parse(raw);
    if (!isRecord(value)) {
      console.warn("pinglet: 本地设置不是对象，已使用默认值");
      return fallback;
    }

    const theme = value.theme;
    const fontSize = value.fontSize;

    return {
      theme: theme === "light" || theme === "dark" ? theme : fallback.theme,
      fontSize:
        typeof fontSize === "string" && FONT_SIZES.has(fontSize as FontSize)
          ? (fontSize as FontSize)
          : fallback.fontSize,
      transparency:
        typeof value.transparency === "number" && Number.isFinite(value.transparency)
          ? normalizePanelTransparency(value.transparency)
          : fallback.transparency,
      reminders: parseReminders(value.reminders),
      closeToTray:
        typeof value.closeToTray === "boolean" ? value.closeToTray : fallback.closeToTray,
      clickThrough:
        typeof value.clickThrough === "boolean" ? value.clickThrough : fallback.clickThrough,
      autoStart: typeof value.autoStart === "boolean" ? value.autoStart : fallback.autoStart,
      shortcut:
        typeof value.shortcut === "string"
          ? value.shortcut.trim().slice(0, SHORTCUT_MAX_LENGTH)
          : fallback.shortcut,
      pinned: typeof value.pinned === "boolean" ? value.pinned : fallback.pinned,
    };
  } catch (error) {
    console.warn("pinglet: 解析本地设置失败，已使用默认值", error);
    return fallback;
  }
}

function loadSettings(): Settings {
  return parseSettings(readStorage(STORAGE_KEY));
}

function persist(): void {
  writeStorage(STORAGE_KEY, JSON.stringify(settings));
}

function notify(): void {
  applyTheme(settings.theme);
  applyFontSize(settings.fontSize);
  applyPanelTransparency(settings.transparency);
  for (const listener of listeners) {
    try {
      listener();
    } catch (error) {
      console.error("pinglet: 设置变更监听器执行失败", error);
    }
  }
}

applyTheme(settings.theme);
applyFontSize(settings.fontSize);
applyPanelTransparency(settings.transparency);
