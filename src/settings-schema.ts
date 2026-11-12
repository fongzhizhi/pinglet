/**
 * @ai-generated deepseek-v4-flash
 * 设置契约与纯解析逻辑：不依赖 DOM / localStorage，供 Vitest 在 node 环境覆盖。
 * 运行时读写与 DOM 应用见 settings.ts。
 */

export type ThemeMode = "dark" | "light" | "auto";

/** 自动主题采样后真正生效的深浅色；CSS 只认这两个值。 */
export type EffectiveTheme = "dark" | "light";

export type FontSize = "small" | "standard" | "large";

export type ReminderKey = "breathing" | "sound" | "shake" | "none";

export type Reminders = Record<ReminderKey, boolean>;

/** 面板行的持续状态动效：异常行呼吸灯 / 进行中行流光。 */
export type PanelEffectKey = "alertPulse" | "workingFlow";

export type PanelEffects = Record<PanelEffectKey, boolean>;

/** 托盘动效：进行中呼吸 / 异常闪烁，只在窗口隐藏或最小化时可见。 */
export type TrayEffectKey = "workingBreathing" | "alertFlicker";

export type TrayEffects = Record<TrayEffectKey, boolean>;

export type Settings = {
  theme: ThemeMode;
  fontSize: FontSize;
  /** 面板背景透明度：0–0.7，0 为完全不透明 */
  transparency: number;
  reminders: Reminders;
  /** 面板行持续动效；关闭后仅保留静态状态色 */
  panelEffects: PanelEffects;
  /** 托盘动效；关闭后对应状态显示静态托盘图标 */
  trayEffects: TrayEffects;
  closeToTray: boolean;
  clickThrough: boolean;
  autoStart: boolean;
  shortcut: string;
  pinned: boolean;
  /** 是否允许在窗口隐藏且出现新告警时弹出应用内通知消息 */
  systemNotifications: boolean;
};

export const STORAGE_KEY = "pinglet.settings.v1";
const DEFAULT_SHORTCUT = "Alt+Shift+P";
export const SHORTCUT_MAX_LENGTH = 64;
const THEME_AUTO: ThemeMode = "auto";
const FONT_SIZE_STANDARD: FontSize = "standard";
export const PANEL_TRANSPARENCY_MIN = 0;
export const PANEL_TRANSPARENCY_MAX = 0.7;
export const PANEL_TRANSPARENCY_STEP = 0.05;
const DEFAULT_PANEL_TRANSPARENCY = PANEL_TRANSPARENCY_MIN;
const FONT_SIZES: ReadonlySet<FontSize> = new Set(["small", "standard", "large"]);
const THEME_MODES: ReadonlySet<ThemeMode> = new Set(["dark", "light", "auto"]);

export const DEFAULT_REMINDERS: Reminders = {
  breathing: true,
  sound: true,
  shake: true,
  none: false,
};

export const DEFAULT_PANEL_EFFECTS: PanelEffects = {
  alertPulse: true,
  workingFlow: true,
};

export const DEFAULT_TRAY_EFFECTS: TrayEffects = {
  workingBreathing: true,
  alertFlicker: true,
};

export function createDefaultSettings(): Settings {
  return {
    theme: THEME_AUTO,
    fontSize: FONT_SIZE_STANDARD,
    transparency: DEFAULT_PANEL_TRANSPARENCY,
    reminders: { ...DEFAULT_REMINDERS },
    panelEffects: { ...DEFAULT_PANEL_EFFECTS },
    trayEffects: { ...DEFAULT_TRAY_EFFECTS },
    closeToTray: true,
    clickThrough: false,
    autoStart: false,
    shortcut: DEFAULT_SHORTCUT,
    pinned: false,
    systemNotifications: true,
  };
}

export function normalizeShortcut(shortcut: string): string {
  return shortcut.trim().slice(0, SHORTCUT_MAX_LENGTH);
}

/** 限制到滑块范围并吸附到 5% 步进，避免浮点误差影响比较与持久化。 */
export function normalizePanelTransparency(transparency: number): number {
  const clamped = Math.min(PANEL_TRANSPARENCY_MAX, Math.max(PANEL_TRANSPARENCY_MIN, transparency));
  const stepped = Math.round(clamped / PANEL_TRANSPARENCY_STEP) * PANEL_TRANSPARENCY_STEP;
  return Math.round(stepped * 100) / 100;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parsePanelEffects(value: unknown): PanelEffects {
  if (!isRecord(value)) {
    return { ...DEFAULT_PANEL_EFFECTS };
  }

  return {
    alertPulse:
      typeof value.alertPulse === "boolean" ? value.alertPulse : DEFAULT_PANEL_EFFECTS.alertPulse,
    workingFlow:
      typeof value.workingFlow === "boolean"
        ? value.workingFlow
        : DEFAULT_PANEL_EFFECTS.workingFlow,
  };
}

function parseTrayEffects(value: unknown): TrayEffects {
  if (!isRecord(value)) {
    return { ...DEFAULT_TRAY_EFFECTS };
  }

  return {
    workingBreathing:
      typeof value.workingBreathing === "boolean"
        ? value.workingBreathing
        : DEFAULT_TRAY_EFFECTS.workingBreathing,
    alertFlicker:
      typeof value.alertFlicker === "boolean"
        ? value.alertFlicker
        : DEFAULT_TRAY_EFFECTS.alertFlicker,
  };
}

/**
 * 三项提醒相互独立；`none` 是「全部关闭」的派生状态，勾选后三项必须为 false。
 * NOTE: 兼容两类历史数据——显式写入的 `none: true`，以及三项全为 false 但缺少 `none` 的记录。
 */
function parseReminders(value: unknown): Reminders {
  if (!isRecord(value)) {
    return { ...DEFAULT_REMINDERS };
  }

  const breathing =
    typeof value.breathing === "boolean" ? value.breathing : DEFAULT_REMINDERS.breathing;
  const sound = typeof value.sound === "boolean" ? value.sound : DEFAULT_REMINDERS.sound;
  const shake = typeof value.shake === "boolean" ? value.shake : DEFAULT_REMINDERS.shake;
  const none = value.none === true || (!breathing && !sound && !shake);
  if (none) {
    return { breathing: false, sound: false, shake: false, none: true };
  }

  return { breathing, sound, shake, none: false };
}

/**
 * 解析持久化设置；损坏 JSON、类型错误与越界值逐字段回退到默认值。
 * NOTE: 主题新增 "auto" 后旧值 "dark"/"light" 保持兼容。
 */
export function parseSettings(raw: string | null): Settings {
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
      theme:
        typeof theme === "string" && THEME_MODES.has(theme as ThemeMode)
          ? (theme as ThemeMode)
          : fallback.theme,
      fontSize:
        typeof fontSize === "string" && FONT_SIZES.has(fontSize as FontSize)
          ? (fontSize as FontSize)
          : fallback.fontSize,
      transparency:
        typeof value.transparency === "number" && Number.isFinite(value.transparency)
          ? normalizePanelTransparency(value.transparency)
          : fallback.transparency,
      reminders: parseReminders(value.reminders),
      panelEffects: parsePanelEffects(value.panelEffects),
      trayEffects: parseTrayEffects(value.trayEffects),
      closeToTray:
        typeof value.closeToTray === "boolean" ? value.closeToTray : fallback.closeToTray,
      clickThrough:
        typeof value.clickThrough === "boolean" ? value.clickThrough : fallback.clickThrough,
      autoStart: typeof value.autoStart === "boolean" ? value.autoStart : fallback.autoStart,
      shortcut:
        typeof value.shortcut === "string" ? normalizeShortcut(value.shortcut) : fallback.shortcut,
      pinned: typeof value.pinned === "boolean" ? value.pinned : fallback.pinned,
      systemNotifications:
        typeof value.systemNotifications === "boolean"
          ? value.systemNotifications
          : fallback.systemNotifications,
    };
  } catch (error) {
    console.warn("pinglet: 解析本地设置失败，已使用默认值", error);
    return fallback;
  }
}
