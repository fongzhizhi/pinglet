/**
 * @ai-generated deepseek-v4-flash
 * Pinglet 看板前端入口：轮询 Rust 侧的 list_sessions，并按四态渲染、提醒与窗口行为。
 */
import { scheduleAutoThemeSample, startAutoTheme, stopAutoTheme } from "./auto-theme";
import { assignFolderColorSlots, folderColorKey, folderTagColor } from "./folder-colors";
import { resolveHeartbeatLevel, type HeartbeatLevel } from "./heartbeat";
import { detectReminderEvent, type AbnormalState } from "./reminder-state";
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
  type BoardState,
  type HiddenSessionSnapshot,
  type ListResult,
  type SessionRow,
} from "./session-state";
import {
  getSettings,
  resetSettings,
  setClickThroughSetting,
  setPinnedSetting,
  setThemeTransitionEnabled,
  subscribeSettings,
  type ReminderKey,
} from "./settings";
import { mountSettingsPanel } from "./settings-panel";
import {
  IS_TAURI,
  activateSession,
  closeWindow,
  hideMainWindowNow,
  keepWindowInsideWorkArea,
  listSessions,
  onOpenSettings,
  onTrayClickThroughChange,
  onWindowHideRequest,
  onWindowMoved,
  onWindowResized,
  onWindowShowRequest,
  onWindowVisibility,
  resetWindowPosition,
  resizeWindow,
  setAutoStart,
  setClickThrough,
  setCloseToTray,
  setActivityPreferences,
  setGlobalShortcut,
  startResizeDragging,
  toMessage,
  type ResizeDirection,
} from "./system";
import {
  beginPanelHide,
  completePanelHide,
  createPanelAnimationState,
  markPanelHidden,
  markPanelShown,
} from "./window-animation";
import {
  DEFAULT_WINDOW_HEIGHT,
  DEFAULT_WINDOW_WIDTH,
  WINDOW_MIN_HEIGHT,
  clampWindowHeight,
  clampWindowWidth,
  maxWindowHeight,
  normalizeStoredWindowSize,
  type StoredWindowSize,
} from "./window-size";

const REFRESH_INTERVAL_MS = 2000;
const HEARTBEAT_INTERVAL_MS = 1000;
// v2：默认宽度由 280 调整为 320，换键让旧存档不再覆盖新默认宽度
const WINDOW_SIZE_STORAGE_KEY = "pinglet.window.v2";
const WINDOW_SIZE_SAVE_DELAY_MS = 350;
const WINDOW_RESIZE_SETTLE_MS = 500;
const WINDOW_SIZE_EPSILON = 2;
const RESET_RELOAD_DELAY_MS = 350;
const PANEL_BORDER_HEIGHT = 2;
const SETTINGS_TOP_OFFSET = 44;
const SETTINGS_BOTTOM_GAP = 8;
const SETTINGS_BORDER_HEIGHT = 2;
const TOOLTIP_OFFSET = 12;
const TOOLTIP_EDGE_MARGIN = 8;
const TOOLTIP_SHOW_DELAY_MS = 1500;
const LOGO_TOOLTIP_GAP = 8;
const SCROLLBAR_HIDE_DELAY_MS = 900;
// 呼吸灯动画：1.1s 一轮；异常提醒三轮、完成提示一轮，留 100ms 余量避免末轮被提前移除
const BREATHING_CYCLE_MS = 1100;
const ALERT_BREATHING_CYCLES = 3;
const COMPLETION_BREATHING_CYCLES = 1;
const BREATHING_TIMEOUT_MARGIN_MS = 100;
const SHAKE_DURATION_MS = 620;
const SHAKE_ONCE_DURATION_MS = 320;
const FLASH_DURATION_MS = 700;
const NOTICE_DURATION_MS = 2000;
const SOUND_START_FREQUENCY = 880;
const SOUND_END_FREQUENCY = 660;
const SOUND_DURATION_SECONDS = 0.18;
const SOUND_GAIN = 0.2;
const SOUND_RAMP_SECONDS = 0.01;
// 首屏稳定后再开启主题过渡，避开自动主题的首次采样修正
const THEME_TRANSITION_ENABLE_DELAY_MS = 1000;
// 与 styles.css 的 --panel-hide/show-duration 保持一致；动画期间暂停高度测量
const PANEL_HIDE_ANIMATION_MS = 160;
const PANEL_SHOW_ANIMATION_MS = 220;
// 过渡结束后留一点余量再测量，避免 transform 中间值污染 getBoundingClientRect
const PANEL_ANIMATION_SETTLE_MS = 40;
const PREFERS_REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

const STATE_CLASSES = BOARD_STATES.map((state) => STATE_META[state].className);

// 第二行右侧的入口图标：VSCode 蓝 / 终端红用于区分入口，入口文字仍在悬浮详情中展示
const VSCODE_ICON_SVG =
  '<svg viewBox="0 0 24 24" aria-hidden="true"><path fill="currentColor" d="M23.15 2.587 18.21.21a1.494 1.494 0 0 0-1.705.29l-9.46 8.63-4.12-3.128a.999.999 0 0 0-1.276.057L.327 7.261A1 1 0 0 0 .326 8.74L3.899 12 .326 15.26a1 1 0 0 0 .001 1.479L1.65 17.94a.999.999 0 0 0 1.276.057l4.12-3.128 9.46 8.63a1.492 1.492 0 0 0 1.704.29l4.942-2.377A1.5 1.5 0 0 0 24 20.06V3.939a1.5 1.5 0 0 0-.85-1.352zm-5.146 14.861L10.826 12l7.178-5.448v10.896z"/></svg>';
const CLI_ICON_SVG =
  '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="1.5" y="1.5" width="21" height="21" rx="4.2" fill="none" stroke="currentColor" stroke-width="3.6"/><path fill="none" stroke="currentColor" stroke-width="3.6" stroke-linecap="round" stroke-linejoin="round" d="M6.6 8.2 10.3 11.8 6.6 15.4M12.9 15.4h5.9"/></svg>';

// 无边框窗口的 8 个缩放方向，名称与 Tauri ResizeDirection 一致
const RESIZE_DIRECTIONS: ReadonlySet<ResizeDirection> = new Set([
  "East",
  "North",
  "NorthEast",
  "NorthWest",
  "South",
  "SouthEast",
  "SouthWest",
  "West",
]);

const STATE_COLOR_VARS: Readonly<Record<BoardState, string>> = {
  blocked: "--color-state-pending",
  interrupted: "--color-state-interrupted",
  working: "--color-state-running",
  done: "--color-state-done",
};

type RowView = {
  element: HTMLDivElement;
  dot: HTMLSpanElement;
  stateLabel: HTMLSpanElement;
  name: HTMLSpanElement;
  folder: HTMLSpanElement;
  vscodeIcon: HTMLSpanElement;
  cliIcon: HTMLSpanElement;
  duration: HTMLSpanElement;
};

type BreathingKind = "alert" | "completion";

if (!IS_TAURI) {
  document.body.classList.add("browser-preview");
}

const panelElement = requireElement<HTMLElement>("#panel");
const panelStageElement = requireElement<HTMLDivElement>("#panel-stage");
const boardHeaderElement = requireElement<HTMLElement>("#board-header");
const listElement = requireElement<HTMLDivElement>("#session-list");
const errorBarElement = requireElement<HTMLDivElement>("#error-bar");
const summaryElement = requireElement<HTMLSpanElement>("#summary");
const logoElement = requireElement<SVGElement>(".logo");
const heartbeatElement = requireElement<HTMLSpanElement>("#heartbeat");
const heartbeatTextElement = requireElement<HTMLSpanElement>("#heartbeat-text");
const footerGapElement = requireElement<HTMLElement>(".footer-gap");
const pinButton = requireElement<HTMLButtonElement>("#pin-btn");
const settingsButton = requireElement<HTMLButtonElement>("#settings-btn");
const closeButton = requireElement<HTMLButtonElement>("#close-btn");
const closeIconElement = requireElement<SVGElement>("#close-icon");
const minimizeIconElement = requireElement<SVGElement>("#minimize-icon");
const settingsBackdropElement = requireElement<HTMLDivElement>("#settings-backdrop");
const settingsPanelElement = requireElement<HTMLElement>("#settings-panel");
const resizeLayerElement = requireElement<HTMLDivElement>("#resize-layer");
const tooltipElement = requireElement<HTMLDivElement>("#tooltip");
const tooltipTitleElement = requireElement<HTMLDivElement>("#tooltip-title");
const tooltipBodyElement = requireElement<HTMLDivElement>("#tooltip-body");
const logoTooltipElement = requireElement<HTMLDivElement>("#logo-tooltip");

const rowViews = new Map<string, RowView>();
const rowsByKey = new Map<string, SessionRow>();
let folderColorSlots = new Map<string, number>();

let isRefreshing = false;
let lastSuccessfulRefreshAt: number | null = null;
// 显隐以 Rust 的 pinglet://window-visibility 为权威来源，页面可见性只作兜底
let isWindowActive = true;
let lastHeartbeatLevel: HeartbeatLevel | null = null;
let lastSummarySignature = "";

const storedWindowSize = readStoredWindowSize();
let preferredWindowWidth = clampWindowWidth(storedWindowSize?.width ?? DEFAULT_WINDOW_WIDTH);
let preferredWindowHeight = clampWindowHeight(
  storedWindowSize?.height ?? DEFAULT_WINDOW_HEIGHT,
  getMaxWindowHeight(),
);
let lastAppliedWindowWidth = preferredWindowWidth;
let lastAppliedWindowHeight = Math.max(WINDOW_MIN_HEIGHT, window.innerHeight);
let lastUserResizeAt = 0;
let autoResizeTimer: number | undefined;
let windowSizeSaveTimer: number | undefined;
// 首次渲染可能早于启动尺寸恢复完成，自动调整需等它落地，避免互相覆盖
let initialWindowSizeReady: Promise<void> = Promise.resolve();

let isSettingsOpen = false;
// NOTE: 设置面板内开启穿透时先挂起，等面板关闭后再应用，否则面板会立刻失去点击能力
let isClickThroughPending = false;
// 已完成会话的临时隐藏集合只驻留内存，不写入 localStorage；状态变化后自动清理恢复
const hiddenSessions = new Map<string, HiddenSessionSnapshot>();
let latestRows: SessionRow[] = [];
let rowMenuElement: HTMLDivElement | null = null;
let rowMenuButton: HTMLButtonElement | null = null;
let rowMenuTargetKey: string | null = null;
// 设置面板挂载后回填，用于同步「恢复临时隐藏」按钮的计数与禁用态
let hiddenCountListener: ((count: number) => void) | null = null;
let previousStates = new Map<string, BoardState>();
let hasRenderedSessions = false;
let emptyElement: HTMLDivElement | null = null;
let hoveredRowKey: string | null = null;
let tooltipTimer: number | undefined;
let logoHoverTimer: number | undefined;
let isLogoHovered = false;
let pointerX = 0;
let pointerY = 0;
let scrollbarTimer: number | undefined;
let noticeTimer: number | undefined;
let currentErrors: string[] = [];
let breathingTimer: number | undefined;
let breathingKind: BreathingKind | null = null;
let shakeTimer: number | undefined;
let audioContext: AudioContext | null = null;
// 窗口显隐动画状态：收起 / 展开期间暂停内容高度测量，避免 transform 影响测量结果
const panelAnimation = createPanelAnimationState();
let panelHideTimer: number | undefined;
let panelShowTimer: number | undefined;
let isPanelAnimating = false;

function requireElement<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) {
    throw new Error(`缺少 DOM 节点: ${selector}`);
  }
  return element;
}

function createRowView(): RowView {
  const element = document.createElement("div");
  element.className = "session-row";
  element.setAttribute("role", "listitem");

  const dot = document.createElement("span");
  dot.className = "dot";
  dot.setAttribute("aria-hidden", "true");

  const stateLabel = document.createElement("span");
  stateLabel.className = "state-label";

  const name = document.createElement("span");
  name.className = "session-name";

  const folder = document.createElement("span");
  folder.className = "cwd";

  // NOTE: 两种入口图标各自持有固定节点与固定颜色，行复用时只切换 hidden。
  // 不在运行期替换 innerHTML / 切换 color，避免 WebView2 复用旧合成层纹理导致颜色残留。
  const vscodeIcon = document.createElement("span");
  vscodeIcon.className = "entry-icon";
  vscodeIcon.dataset.kind = "vs-code";
  vscodeIcon.setAttribute("aria-hidden", "true");
  vscodeIcon.innerHTML = VSCODE_ICON_SVG;
  vscodeIcon.hidden = true;

  const cliIcon = document.createElement("span");
  cliIcon.className = "entry-icon";
  cliIcon.dataset.kind = "cli";
  cliIcon.setAttribute("aria-hidden", "true");
  cliIcon.innerHTML = CLI_ICON_SVG;
  cliIcon.hidden = true;

  const duration = document.createElement("span");
  duration.className = "duration";

  // 双行布局：第一行状态、目录标签、入口图标与时长，第二行标题独占整行
  // NOTE: 目录是稳定且短的会话识别锚点，优先于 AI 生成的标题；入口文字只在悬浮详情中展示
  const mainLine = document.createElement("div");
  mainLine.className = "session-main";
  mainLine.append(stateLabel, folder, vscodeIcon, cliIcon, duration);

  const content = document.createElement("div");
  content.className = "session-content";
  content.append(mainLine, name);

  element.append(dot, content);

  element.addEventListener("mouseenter", (event) => {
    if (rowsByKey.get(element.dataset.key ?? "") === undefined) {
      return;
    }
    hideLogoTooltip();
    hoveredRowKey = element.dataset.key ?? null;
    pointerX = event.clientX;
    pointerY = event.clientY;
    scheduleTooltip();
  });
  element.addEventListener("mousemove", (event) => {
    pointerX = event.clientX;
    pointerY = event.clientY;
    if (!tooltipElement.hidden) {
      positionTooltip(pointerX, pointerY);
    }
  });
  element.addEventListener("mouseleave", () => {
    hideTooltip();
  });
  element.addEventListener("dblclick", () => {
    // 双击进入激活流程后收起悬浮面板，避免遮住行内容
    hideTooltip();
    const row = rowsByKey.get(element.dataset.key ?? "");
    if (row !== undefined) {
      void activateRow(row, element);
    }
  });
  element.addEventListener("keydown", (event) => {
    const row = rowsByKey.get(element.dataset.key ?? "");
    if (row === undefined) {
      return;
    }
    if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
      if (!canHideSession(classify(row))) {
        return;
      }
      event.preventDefault();
      const rect = element.getBoundingClientRect();
      showRowMenu(rect.left + TOOLTIP_OFFSET, rect.top + TOOLTIP_OFFSET, getRowKey(row));
      return;
    }
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }
    event.preventDefault();
    void activateRow(row, element);
  });
  element.addEventListener("contextmenu", (event) => {
    const row = rowsByKey.get(element.dataset.key ?? "");
    if (row === undefined || !canHideSession(classify(row))) {
      return;
    }
    event.preventDefault();
    hideTooltip();
    hideLogoTooltip();
    showRowMenu(event.clientX, event.clientY, getRowKey(row));
  });

  return {
    element,
    dot,
    stateLabel,
    name,
    folder,
    vscodeIcon,
    cliIcon,
    duration,
  };
}

function updateRowView(
  view: RowView,
  row: SessionRow,
  state: BoardState,
  now: number,
  folderColors: Map<string, number>,
): void {
  const meta = STATE_META[state];
  const key = getRowKey(row);

  for (const className of STATE_CLASSES) {
    view.element.classList.remove(className);
  }
  view.element.classList.add(meta.className);
  view.element.dataset.key = key;
  view.element.dataset.state = state;

  view.stateLabel.textContent = meta.label;
  view.name.textContent = row.title;

  const folder = directoryName(row.cwd);
  view.folder.textContent = folder;
  view.folder.style.setProperty(
    "--tag-color",
    folderTagColor(folderColorKey(row.cwd), folderColors),
  );

  const entrypoint = entrypointLabel(row.entrypoint);
  const iconKind = entrypoint === "vs-code" || entrypoint === "cli" ? entrypoint : null;
  view.vscodeIcon.hidden = iconKind !== "vs-code";
  view.cliIcon.hidden = iconKind !== "cli";

  view.duration.textContent = describeDuration(row, state, now);

  const isDisabled = row.cwd === null;
  view.element.classList.toggle("is-disabled", isDisabled);
  view.element.tabIndex = isDisabled ? -1 : 0;
  view.element.setAttribute("aria-disabled", String(isDisabled));
  view.element.setAttribute("aria-label", `${meta.label}，${folder}，${row.title}`);
}

function renderRows(rows: SessionRow[]): void {
  latestRows = rows;
  // 先按快照清理失效的隐藏项，再过滤；隐藏行不参与排序、汇总、测高与状态提醒
  const visibleRows = applyHiddenSessions(rows, hiddenSessions);
  const now = Date.now();
  const sortedRows = sortRows(visibleRows);
  const desiredKeys = new Set(sortedRows.map((row) => getRowKey(row)));
  folderColorSlots = assignFolderColorSlots(
    sortedRows.map((row) => folderColorKey(row.cwd)),
    folderColorSlots,
  );

  for (const [key, view] of rowViews) {
    if (desiredKeys.has(key)) {
      continue;
    }
    view.element.remove();
    rowViews.delete(key);
  }
  if (hoveredRowKey !== null && !desiredKeys.has(hoveredRowKey)) {
    hideTooltip();
  }
  if (rowMenuTargetKey !== null && !desiredKeys.has(rowMenuTargetKey)) {
    hideRowMenu();
  }

  rowsByKey.clear();
  for (const row of sortedRows) {
    const key = getRowKey(row);
    let view = rowViews.get(key);
    if (view === undefined) {
      view = createRowView();
      rowViews.set(key, view);
    }
    rowsByKey.set(key, row);
    updateRowView(view, row, classify(row), now, folderColorSlots);
  }

  const currentOrder = [...listElement.children]
    .filter((child): child is HTMLElement => child instanceof HTMLElement)
    .map((child) => child.dataset.key ?? "");
  const desiredOrder = sortedRows.map((row) => getRowKey(row));
  const orderChanged =
    currentOrder.length !== desiredOrder.length ||
    currentOrder.some((key, index) => key !== desiredOrder[index]);

  if (sortedRows.length === 0) {
    if (emptyElement === null) {
      emptyElement = document.createElement("div");
      emptyElement.className = "empty";
    }
    emptyElement.textContent =
      hiddenSessions.size > 0 ? `已临时隐藏 ${hiddenSessions.size} 个已完成会话` : "暂无活跃会话";
    if (!emptyElement.isConnected) {
      listElement.append(emptyElement);
    }
  } else if (emptyElement !== null && emptyElement.isConnected) {
    emptyElement.remove();
  }

  if (orderChanged) {
    for (const key of desiredOrder) {
      const view = rowViews.get(key);
      if (view !== undefined) {
        listElement.append(view.element);
      }
    }
  }

  const blockedCount = sortedRows.filter((row) => classify(row) === "blocked").length;
  const interruptedCount = sortedRows.filter((row) => classify(row) === "interrupted").length;
  const workingCount = sortedRows.filter((row) => classify(row) === "working").length;
  renderSummary(blockedCount, interruptedCount, workingCount);
  hiddenCountListener?.(hiddenSessions.size);

  const { event: reminderEvent, nextStates } = detectReminderEvent(
    sortedRows,
    previousStates,
    hasRenderedSessions,
  );
  previousStates = nextStates;
  hasRenderedSessions = true;
  if (reminderEvent !== null) {
    if (reminderEvent.kind === "alert") {
      triggerAlertReminder(reminderEvent.state);
    } else {
      triggerCompletionReminder();
    }
  }
}

function renderCurrentRows(): void {
  renderRows(latestRows);
  window.requestAnimationFrame(() => {
    void resizeWindowToContent();
  });
}

function ensureRowMenu(): { menu: HTMLDivElement; button: HTMLButtonElement } {
  if (rowMenuElement !== null && rowMenuButton !== null) {
    return { menu: rowMenuElement, button: rowMenuButton };
  }

  const menu = document.createElement("div");
  menu.className = "row-menu";
  menu.setAttribute("role", "menu");
  menu.hidden = true;

  const button = document.createElement("button");
  button.type = "button";
  button.className = "row-menu-item";
  button.setAttribute("role", "menuitem");
  button.textContent = "临时隐藏";
  button.addEventListener("click", () => {
    const key = rowMenuTargetKey;
    hideRowMenu();
    if (key !== null) {
      hideSession(key);
    }
  });

  menu.append(button);
  document.body.append(menu);
  rowMenuElement = menu;
  rowMenuButton = button;
  return { menu, button };
}

function showRowMenu(x: number, y: number, key: string): void {
  const { menu, button } = ensureRowMenu();
  hideTooltip();
  hideLogoTooltip();
  rowMenuTargetKey = key;
  menu.hidden = false;
  // 先显示再测量，才能按实际尺寸在窗口边缘翻转菜单
  const rect = menu.getBoundingClientRect();
  const left = x + rect.width > window.innerWidth - TOOLTIP_EDGE_MARGIN ? x - rect.width : x;
  const top = y + rect.height > window.innerHeight - TOOLTIP_EDGE_MARGIN ? y - rect.height : y;
  menu.style.left = `${Math.max(TOOLTIP_EDGE_MARGIN, left)}px`;
  menu.style.top = `${Math.max(TOOLTIP_EDGE_MARGIN, top)}px`;
  button.focus();
}

function hideRowMenu(): void {
  rowMenuTargetKey = null;
  if (rowMenuElement !== null) {
    rowMenuElement.hidden = true;
  }
}

function hideSession(key: string): void {
  const row = rowsByKey.get(key);
  if (row === undefined || !canHideSession(classify(row))) {
    return;
  }
  hiddenSessions.set(key, createHiddenSessionSnapshot(row));
  hideTooltip();
  hideLogoTooltip();
  renderCurrentRows();
}

/** 清空全部临时隐藏项并返回恢复条数，供设置面板展示结果。 */
function restoreHiddenSessions(): number {
  const count = hiddenSessions.size;
  if (count === 0) {
    return 0;
  }
  hiddenSessions.clear();
  hideRowMenu();
  renderCurrentRows();
  return count;
}

function renderSummary(blockedCount: number, interruptedCount: number, workingCount: number): void {
  // 计数没变时跳过 DOM 重建，轮询每 2 秒触发也不产生无意义重排
  const signature = `${blockedCount}|${interruptedCount}|${workingCount}`;
  if (signature === lastSummarySignature) {
    return;
  }
  lastSummarySignature = signature;
  summaryElement.replaceChildren();

  if (blockedCount > 0) {
    const chip = document.createElement("span");
    chip.className = "summary-chip summary-blocked";
    chip.textContent = `${blockedCount} ${STATE_META.blocked.label}`;
    summaryElement.append(chip);
  }

  if (interruptedCount > 0) {
    const chip = document.createElement("span");
    chip.className = "summary-chip summary-interrupted";
    chip.textContent = `${interruptedCount} ${STATE_META.interrupted.label}`;
    summaryElement.append(chip);
  }

  if (workingCount > 0) {
    const chip = document.createElement("span");
    chip.className = "summary-chip summary-working";
    chip.textContent = `${workingCount} ${STATE_META.working.label}`;
    summaryElement.append(chip);
  }
}

function setAlertColor(state: BoardState): void {
  panelElement.style.setProperty("--alert-color", `var(${STATE_COLOR_VARS[state]})`);
}

function playBreathingEffect(cycles: number, kind: BreathingKind): void {
  panelElement.classList.remove("is-breathing");
  panelElement.style.setProperty("--breathing-cycles", String(cycles));
  void panelElement.offsetWidth;
  panelElement.classList.add("is-breathing");
  breathingKind = kind;
  if (breathingTimer !== undefined) {
    window.clearTimeout(breathingTimer);
  }
  breathingTimer = window.setTimeout(
    () => {
      panelElement.classList.remove("is-breathing");
      breathingTimer = undefined;
      breathingKind = null;
    },
    BREATHING_CYCLE_MS * cycles + BREATHING_TIMEOUT_MARGIN_MS,
  );
}

function playShakeEffect(kind: BreathingKind = "alert"): void {
  const className = kind === "alert" ? "is-shaking" : "is-shaking-once";
  const duration = kind === "alert" ? SHAKE_DURATION_MS : SHAKE_ONCE_DURATION_MS;

  panelElement.classList.remove("is-shaking", "is-shaking-once");
  void panelElement.offsetWidth;
  panelElement.classList.add(className);
  if (shakeTimer !== undefined) {
    window.clearTimeout(shakeTimer);
  }
  shakeTimer = window.setTimeout(() => {
    panelElement.classList.remove(className);
    shakeTimer = undefined;
  }, duration);
}

function triggerAlertReminder(state: AbnormalState): void {
  const { reminders } = getSettings();

  // 告警色始终更新；呼吸灯、音效与抖动各自独立开关
  setAlertColor(state);
  if (reminders.breathing) {
    playBreathingEffect(ALERT_BREATHING_CYCLES, "alert");
  }
  if (reminders.shake) {
    playShakeEffect();
  }
  if (reminders.sound) {
    playNotificationSound();
  }
}

/** 完成提示：呼吸灯用进行中语义色闪烁一次，音效响一次，窗口抖动一次。 */
function triggerCompletionReminder(): void {
  const { reminders } = getSettings();
  // 异常提醒进行中时不让完成呼吸打断它的颜色与轮次，但音效与抖动照常
  if (reminders.breathing && breathingKind !== "alert") {
    setAlertColor("working");
    playBreathingEffect(COMPLETION_BREATHING_CYCLES, "completion");
  }
  if (reminders.shake) {
    playShakeEffect("completion");
  }
  if (reminders.sound) {
    playNotificationSound();
  }
}

/** 设置面板勾选提醒项时即时演示效果，使用待处理色以便在两种主题下都醒目。 */
function previewReminder(key: Exclude<ReminderKey, "none">): void {
  setAlertColor("blocked");
  if (key === "breathing") {
    playBreathingEffect(ALERT_BREATHING_CYCLES, "alert");
    return;
  }
  if (key === "shake") {
    playShakeEffect();
    return;
  }
  playNotificationSound();
}

function unlockAudio(): void {
  if (typeof AudioContext === "undefined") {
    return;
  }
  audioContext ??= new AudioContext();
  void audioContext.resume().catch((error: unknown) => {
    console.warn("pinglet: 解锁音频失败", error);
  });
}

function playNotificationSound(): void {
  if (typeof AudioContext === "undefined") {
    return;
  }

  audioContext ??= new AudioContext();
  void audioContext
    .resume()
    .then(() => {
      const context = audioContext;
      if (context === null) {
        return;
      }
      const startAt = context.currentTime;
      const oscillator = context.createOscillator();
      const gain = context.createGain();

      oscillator.type = "sine";
      oscillator.frequency.setValueAtTime(SOUND_START_FREQUENCY, startAt);
      oscillator.frequency.exponentialRampToValueAtTime(
        SOUND_END_FREQUENCY,
        startAt + SOUND_DURATION_SECONDS,
      );
      gain.gain.setValueAtTime(0.0001, startAt);
      gain.gain.exponentialRampToValueAtTime(SOUND_GAIN, startAt + SOUND_RAMP_SECONDS);
      gain.gain.exponentialRampToValueAtTime(0.0001, startAt + SOUND_DURATION_SECONDS);

      oscillator.connect(gain);
      gain.connect(context.destination);
      oscillator.start(startAt);
      oscillator.stop(startAt + SOUND_DURATION_SECONDS);
    })
    .catch((error: unknown) => {
      console.warn("pinglet: 提示音播放失败", error);
    });
}

function scheduleTooltip(): void {
  // NOTE: 悬停 1.5 秒才弹出，避免鼠标划过会话行时挡住双击/点击
  if (hoveredRowKey === null || !tooltipElement.hidden) {
    return;
  }
  if (tooltipTimer !== undefined) {
    window.clearTimeout(tooltipTimer);
  }
  tooltipTimer = window.setTimeout(() => {
    tooltipTimer = undefined;
    const key = hoveredRowKey;
    if (key === null || !tooltipElement.hidden) {
      return;
    }
    const row = rowsByKey.get(key);
    if (row === undefined) {
      return;
    }
    showTooltip(row, pointerX, pointerY);
  }, TOOLTIP_SHOW_DELAY_MS);
}

function showTooltip(row: SessionRow, x: number, y: number): void {
  const state = classify(row);
  const tooltip = buildTooltip(row, state);

  tooltipTitleElement.textContent = tooltip.title;
  const lineElements = tooltip.lines.map((line) => {
    const element = document.createElement("div");
    element.className = "tooltip-line";
    element.textContent = line;
    return element;
  });
  if (tooltip.hint !== null) {
    const hint = document.createElement("div");
    hint.className = "tooltip-hint";
    hint.textContent = tooltip.hint;
    lineElements.push(hint);
  }
  tooltipBodyElement.replaceChildren(...lineElements);
  tooltipElement.hidden = false;
  positionTooltip(x, y);
}

function positionTooltip(x: number, y: number): void {
  if (tooltipElement.hidden) {
    return;
  }

  const rect = tooltipElement.getBoundingClientRect();
  let left = x + TOOLTIP_OFFSET;
  let top = y + TOOLTIP_OFFSET;

  if (left + rect.width > window.innerWidth - TOOLTIP_EDGE_MARGIN) {
    left = x - rect.width - TOOLTIP_OFFSET;
  }
  if (top + rect.height > window.innerHeight - TOOLTIP_EDGE_MARGIN) {
    top = y - rect.height - TOOLTIP_OFFSET;
  }

  tooltipElement.style.left = `${Math.max(TOOLTIP_EDGE_MARGIN, left)}px`;
  tooltipElement.style.top = `${Math.max(TOOLTIP_EDGE_MARGIN, top)}px`;
}

function hideTooltip(): void {
  hoveredRowKey = null;
  if (tooltipTimer !== undefined) {
    window.clearTimeout(tooltipTimer);
    tooltipTimer = undefined;
  }
  tooltipElement.hidden = true;
}

/** 悬停 logo 1.5 秒后展示品牌与数据刷新状态，避免常驻的心跳信息占用面板空间。 */
function scheduleLogoTooltip(): void {
  if (logoHoverTimer !== undefined) {
    window.clearTimeout(logoHoverTimer);
  }
  logoHoverTimer = window.setTimeout(() => {
    logoHoverTimer = undefined;
    if (!isLogoHovered) {
      return;
    }
    // 显示瞬间同步一次，保证时间与颜色是最新值
    updateHeartbeat(true);
    logoTooltipElement.hidden = false;
    positionLogoTooltip();
  }, TOOLTIP_SHOW_DELAY_MS);
}

function positionLogoTooltip(): void {
  if (logoTooltipElement.hidden) {
    return;
  }

  const logoRect = logoElement.getBoundingClientRect();
  const tooltipRect = logoTooltipElement.getBoundingClientRect();
  let left = logoRect.left;
  let top = logoRect.bottom + LOGO_TOOLTIP_GAP;

  // 下方空间不足时翻到 logo 上方，左右按窗口边缘夹取
  if (top + tooltipRect.height > window.innerHeight - TOOLTIP_EDGE_MARGIN) {
    top = logoRect.top - tooltipRect.height - LOGO_TOOLTIP_GAP;
  }
  left = Math.min(left, window.innerWidth - tooltipRect.width - TOOLTIP_EDGE_MARGIN);

  logoTooltipElement.style.left = `${Math.max(TOOLTIP_EDGE_MARGIN, left)}px`;
  logoTooltipElement.style.top = `${Math.max(TOOLTIP_EDGE_MARGIN, top)}px`;
}

function hideLogoTooltip(): void {
  isLogoHovered = false;
  if (logoHoverTimer !== undefined) {
    window.clearTimeout(logoHoverTimer);
    logoHoverTimer = undefined;
  }
  logoTooltipElement.hidden = true;
}

/** 滚动时短暂显示滚动条；悬停列表也可通过 CSS 显示。 */
function revealScrollbar(): void {
  listElement.classList.add("is-scrolling");
  if (scrollbarTimer !== undefined) {
    window.clearTimeout(scrollbarTimer);
  }
  scrollbarTimer = window.setTimeout(() => {
    listElement.classList.remove("is-scrolling");
    scrollbarTimer = undefined;
  }, SCROLLBAR_HIDE_DELAY_MS);
}

function flashRow(element: HTMLElement): void {
  element.classList.remove("is-flashing");
  void element.offsetWidth;
  element.classList.add("is-flashing");
  window.setTimeout(() => {
    element.classList.remove("is-flashing");
  }, FLASH_DURATION_MS);
}

async function activateRow(row: SessionRow, element: HTMLElement): Promise<void> {
  if (row.cwd === null) {
    flashRow(element);
    showNotice("该会话没有可打开的目录");
    return;
  }
  if (sessionAction(row) === null) {
    flashRow(element);
    showNotice("该会话不支持双击打开（仅支持 VSCode / CLI）");
    return;
  }
  if (!IS_TAURI) {
    flashRow(element);
    showNotice("浏览器预览模式：双击激活 VSCode / Git Bash 不可用");
    return;
  }

  try {
    await activateSession(row.cwd, row.entrypoint, row.name, row.pid);
  } catch (error) {
    flashRow(element);
    showNotice(toMessage(error));
  }
}

function renderErrors(errors: string[]): void {
  currentErrors = errors;
  if (errors.length === 0) {
    // 临时提示未到期时继续展示，由 showNotice 的定时器负责清除
    if (noticeTimer !== undefined) {
      return;
    }
    clearNoticeBar();
    return;
  }

  // 持续的会话读取异常优先于临时提示
  if (noticeTimer !== undefined) {
    window.clearTimeout(noticeTimer);
    noticeTimer = undefined;
  }
  errorBarElement.textContent = `! ${errors.length} 条异常`;
  errorBarElement.title = errors.join("\n");
  errorBarElement.classList.add("visible");
}

function clearNoticeBar(): void {
  errorBarElement.classList.remove("visible");
  errorBarElement.textContent = "";
  errorBarElement.removeAttribute("title");
  window.requestAnimationFrame(() => {
    void resizeWindowToContent();
  });
}

function showNotice(message: string): void {
  errorBarElement.textContent = message;
  errorBarElement.title = message;
  errorBarElement.classList.add("visible");

  if (noticeTimer !== undefined) {
    window.clearTimeout(noticeTimer);
  }
  noticeTimer = window.setTimeout(() => {
    noticeTimer = undefined;
    renderErrors(currentErrors);
  }, NOTICE_DURATION_MS);

  window.requestAnimationFrame(() => {
    void resizeWindowToContent();
  });
}

function formatClockTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString("zh-CN", { hour12: false });
}

/** 心跳每秒计算一次；tooltip 收起时只写等级，避免每秒重写文本节点与辅助属性。 */
function updateHeartbeat(force = false): void {
  const level = resolveHeartbeatLevel(lastSuccessfulRefreshAt, Date.now());

  if (level !== lastHeartbeatLevel) {
    lastHeartbeatLevel = level;
    heartbeatElement.dataset.level = level;
  }

  // 秒级时间仅在用户悬停查看时同步，展示瞬间由 scheduleLogoTooltip 强制刷新
  if (!force && logoTooltipElement.hidden) {
    return;
  }

  const text =
    lastSuccessfulRefreshAt === null
      ? "尚未刷新"
      : `刷新于 ${formatClockTime(lastSuccessfulRefreshAt)}`;
  heartbeatElement.title = text;
  heartbeatElement.setAttribute("aria-label", text);
  heartbeatTextElement.textContent = text;
}

function getMaxWindowHeight(): number {
  return maxWindowHeight(window.screen.availHeight);
}

function readStoredWindowSize(): StoredWindowSize | null {
  try {
    const raw = window.localStorage.getItem(WINDOW_SIZE_STORAGE_KEY);
    if (raw === null) {
      return null;
    }
    const value: unknown = JSON.parse(raw);
    return normalizeStoredWindowSize(value);
  } catch (error) {
    console.warn("pinglet: 读取窗口尺寸失败", error);
    return null;
  }
}

function saveWindowSizeSoon(): void {
  if (windowSizeSaveTimer !== undefined) {
    window.clearTimeout(windowSizeSaveTimer);
  }
  windowSizeSaveTimer = window.setTimeout(() => {
    windowSizeSaveTimer = undefined;
    try {
      window.localStorage.setItem(
        WINDOW_SIZE_STORAGE_KEY,
        JSON.stringify({
          width: preferredWindowWidth,
          height: preferredWindowHeight,
        }),
      );
    } catch (error) {
      console.warn("pinglet: 保存窗口尺寸失败", error);
    }
  }, WINDOW_SIZE_SAVE_DELAY_MS);
}

async function applyWindowSize(width: number, height: number): Promise<void> {
  if (!IS_TAURI) {
    return;
  }
  // 先记录目标值，resize 事件可能在 setSize 的 Promise 完成前到达
  lastAppliedWindowWidth = width;
  lastAppliedWindowHeight = height;
  try {
    await resizeWindow(width, height);
  } catch (error) {
    lastAppliedWindowHeight = 0;
    console.warn("pinglet: 调整窗口尺寸失败", error);
  }
}

/** 用户拖拽缩放后更新偏好；程序自动调整的高度不计入用户偏好。 */
function handleWindowResized(physicalWidth: number, physicalHeight: number): void {
  const scale = window.devicePixelRatio || 1;
  const width = physicalWidth / scale;
  const height = physicalHeight / scale;

  const widthChanged = Math.abs(width - lastAppliedWindowWidth) > WINDOW_SIZE_EPSILON;
  const heightChanged = Math.abs(height - lastAppliedWindowHeight) > WINDOW_SIZE_EPSILON;
  if (!widthChanged && !heightChanged) {
    return;
  }

  lastUserResizeAt = Date.now();
  // 窗口底边变化后采样区域也变了，缩放结束后立即重新判定主题
  scheduleAutoThemeSample();
  // 只拖宽度时不要把当前自动收缩后的高度误记为新的高度上限
  if (widthChanged) {
    preferredWindowWidth = clampWindowWidth(width);
  }
  if (heightChanged) {
    preferredWindowHeight = clampWindowHeight(height, getMaxWindowHeight());
  }
  saveWindowSizeSoon();

  // 拖拽结束后再按内容收缩，拖拽过程中不和系统缩放互相拉扯
  if (autoResizeTimer !== undefined) {
    window.clearTimeout(autoResizeTimer);
  }
  autoResizeTimer = window.setTimeout(() => {
    autoResizeTimer = undefined;
    void resizeWindowToContent(true);
  }, WINDOW_RESIZE_SETTLE_MS);
}

function parseResizeDirection(value: string | undefined): ResizeDirection | null {
  if (value === undefined) {
    return null;
  }
  const direction = value as ResizeDirection;
  return RESIZE_DIRECTIONS.has(direction) ? direction : null;
}

function setupResizeHandles(): void {
  if (!IS_TAURI) {
    resizeLayerElement.hidden = true;
    return;
  }

  for (const handle of resizeLayerElement.querySelectorAll<HTMLElement>(".resize-handle")) {
    const direction = parseResizeDirection(handle.dataset.resizeDirection);
    if (direction === null) {
      continue;
    }
    handle.addEventListener("mousedown", (event) => {
      if (event.button !== 0) {
        return;
      }
      // 阻断顶栏拖拽/文本选择，把这次按下完全交给系统缩放
      event.preventDefault();
      event.stopPropagation();
      void startResizeDragging(direction).catch((error: unknown) => {
        console.warn("pinglet: 开始缩放窗口失败", error);
      });
    });
  }
}

async function applyInitialWindowSize(): Promise<void> {
  const currentHeight = Math.max(WINDOW_MIN_HEIGHT, window.innerHeight);
  const height = clampWindowHeight(
    Math.min(currentHeight, preferredWindowHeight),
    getMaxWindowHeight(),
  );
  await applyWindowSize(preferredWindowWidth, height);

  try {
    await keepWindowInsideWorkArea();
  } catch (error) {
    console.warn("pinglet: 校正窗口位置失败", error);
  }
}

function measureContentHeight(): number {
  const headerHeight = boardHeaderElement.getBoundingClientRect().height;
  const footerGapHeight = footerGapElement.getBoundingClientRect().height;
  const errorHeight = errorBarElement.classList.contains("visible")
    ? errorBarElement.getBoundingClientRect().height
    : 0;
  // scrollHeight 至少等于当前可视高度，无法用于收缩；改为累加行高并补上列表内边距
  const listStyle = window.getComputedStyle(listElement);
  const listPadding =
    Number.parseFloat(listStyle.paddingTop) + Number.parseFloat(listStyle.paddingBottom);
  // 行高可能带小数（如 50.43px），offsetHeight 会逐行截断，累计后窗口偏矮约 2px，
  // 悬停出现滚动条；这里取小数高度并向上取整，保证内容恰好不溢出
  const listHeight =
    [...listElement.children].reduce((sum, child) => {
      return sum + (child instanceof HTMLElement ? child.getBoundingClientRect().height : 0);
    }, 0) + listPadding;

  return Math.ceil(PANEL_BORDER_HEIGHT + headerHeight + listHeight + errorHeight + footerGapHeight);
}

function measureSettingsHeight(): number {
  // 设置面板绝对定位，scrollHeight 不受当前窗口高度裁剪，能拿到自然内容高度
  return (
    PANEL_BORDER_HEIGHT +
    SETTINGS_TOP_OFFSET +
    settingsPanelElement.scrollHeight +
    SETTINGS_BORDER_HEIGHT +
    SETTINGS_BOTTOM_GAP
  );
}

function prefersReducedMotion(): boolean {
  return window.matchMedia(PREFERS_REDUCED_MOTION_QUERY).matches;
}

/** 直接落到收起态且不做过渡；用于窗口已被系统隐藏 / 最小化的场景。 */
function collapsePanelImmediately(): void {
  panelStageElement.classList.add("is-panel-snap");
  panelStageElement.classList.remove("is-panel-collapsing");
  panelStageElement.classList.add("is-panel-collapsed");
  // 强制落定收起样式，避免下次展开时从旧的展开状态插值
  void panelStageElement.offsetWidth;
}

/** 收起动画：结束后回调；reduce-motion 下立即完成。 */
function startPanelHideAnimation(onFinished: () => void): void {
  if (panelShowTimer !== undefined) {
    window.clearTimeout(panelShowTimer);
    panelShowTimer = undefined;
  }
  if (panelHideTimer !== undefined) {
    window.clearTimeout(panelHideTimer);
    panelHideTimer = undefined;
  }
  isPanelAnimating = true;
  hideTooltip();
  hideLogoTooltip();
  hideRowMenu();

  if (prefersReducedMotion()) {
    collapsePanelImmediately();
    isPanelAnimating = false;
    onFinished();
    return;
  }

  panelStageElement.classList.remove("is-panel-snap");
  panelStageElement.classList.add("is-panel-collapsing");
  void panelStageElement.offsetWidth;
  panelStageElement.classList.add("is-panel-collapsed");
  panelHideTimer = window.setTimeout(() => {
    panelHideTimer = undefined;
    isPanelAnimating = false;
    onFinished();
  }, PANEL_HIDE_ANIMATION_MS);
}

function finishPanelShow(): void {
  panelShowTimer = undefined;
  isPanelAnimating = false;
  window.requestAnimationFrame(() => {
    void resizeWindowToContent(true);
  });
}

/** 从收起态展开；首次启动或已展开时不做动画。 */
function expandPanel(): void {
  if (panelHideTimer !== undefined) {
    window.clearTimeout(panelHideTimer);
    panelHideTimer = undefined;
  }
  const shouldAnimate = markPanelShown(panelAnimation);
  if (!shouldAnimate) {
    return;
  }

  if (prefersReducedMotion()) {
    panelStageElement.classList.remove(
      "is-panel-snap",
      "is-panel-collapsing",
      "is-panel-collapsed",
    );
    isPanelAnimating = false;
    window.requestAnimationFrame(() => {
      void resizeWindowToContent(true);
    });
    return;
  }

  isPanelAnimating = true;
  panelStageElement.classList.remove("is-panel-snap", "is-panel-collapsing");
  // 先让收起态在没有过渡的情况下成为起点，再移除类触发展开过渡
  void panelStageElement.offsetWidth;
  panelStageElement.classList.remove("is-panel-collapsed");
  if (panelShowTimer !== undefined) {
    window.clearTimeout(panelShowTimer);
  }
  panelShowTimer = window.setTimeout(
    finishPanelShow,
    PANEL_SHOW_ANIMATION_MS + PANEL_ANIMATION_SETTLE_MS,
  );
}

/** Rust 隐藏入口：先播放收起动画，结束后再真正隐藏；代号过期则放弃。 */
function handleWindowHideRequest(generation: number): void {
  beginPanelHide(panelAnimation, generation);
  startPanelHideAnimation(() => {
    if (!completePanelHide(panelAnimation, generation)) {
      return;
    }
    void hideMainWindowNow(generation).catch((error: unknown) => {
      console.warn("pinglet: 隐藏窗口失败", error);
      expandPanel();
    });
  });
}

/** 窗口已被系统隐藏 / 最小化：保持收起态，下次显示时播放展开。 */
function handleWindowHidden(): void {
  if (panelHideTimer !== undefined) {
    window.clearTimeout(panelHideTimer);
    panelHideTimer = undefined;
  }
  if (panelShowTimer !== undefined) {
    window.clearTimeout(panelShowTimer);
    panelShowTimer = undefined;
  }
  isPanelAnimating = false;
  markPanelHidden(panelAnimation);
  if (!panelStageElement.classList.contains("is-panel-collapsed")) {
    collapsePanelImmediately();
  }
}

async function resizeWindowToContent(force = false): Promise<void> {
  if (!IS_TAURI) {
    return;
  }
  // 显隐动画期间 transform 会污染 getBoundingClientRect，结束后再统一测量
  if (isPanelAnimating || panelAnimation.collapsed) {
    return;
  }
  await initialWindowSizeReady;
  if (isPanelAnimating || panelAnimation.collapsed) {
    return;
  }
  if (!force && Date.now() - lastUserResizeAt < WINDOW_RESIZE_SETTLE_MS) {
    return;
  }

  const contentHeight = isSettingsOpen ? measureSettingsHeight() : measureContentHeight();
  // 用户拖拽的高度是可撑开的上限，内容不足时仍收缩到内容高度
  const maxHeight = Math.min(preferredWindowHeight, getMaxWindowHeight());
  const desiredHeight = clampWindowHeight(Math.min(contentHeight, maxHeight), getMaxWindowHeight());

  // lastApplied* 只记录程序设置的尺寸，用户拖拽不会更新它；
  // 因此这里必须和窗口实际尺寸比较，否则拖高后会被误判为“已到位”而不收缩
  if (
    Math.abs(desiredHeight - window.innerHeight) < WINDOW_SIZE_EPSILON &&
    Math.abs(preferredWindowWidth - window.innerWidth) < WINDOW_SIZE_EPSILON
  ) {
    return;
  }

  await applyWindowSize(preferredWindowWidth, desiredHeight);
}

function render(result: ListResult): void {
  lastSuccessfulRefreshAt = Date.now();
  updateHeartbeat();
  renderRows(result.rows);
  renderErrors(result.errors);
  window.requestAnimationFrame(() => {
    void resizeWindowToContent();
  });
}

async function loadSessions(): Promise<ListResult> {
  if (!IS_TAURI) {
    // 浏览器预览：仅 dev 构建加载模拟数据，生产包回退为明确报错
    if (import.meta.env.DEV) {
      const { createMockResult } = await import("./mock-sessions");
      return createMockResult();
    }
    throw new Error("当前环境不支持读取会话状态");
  }
  return listSessions();
}

async function refresh(): Promise<void> {
  if (isRefreshing) {
    return;
  }

  isRefreshing = true;
  try {
    const result = await loadSessions();
    render(result);
  } catch (error) {
    showNotice(`读取会话状态失败: ${toMessage(error)}`);
    updateHeartbeat();
  } finally {
    isRefreshing = false;
  }
}

function applyPinned(pinned: boolean): void {
  // 两种模式窗口都保持置顶，区别只在顶栏是否可拖动：
  // 文案表示点击后进入的模式——拖动中显示“固定模式”，固定中显示“拖动模式”
  const actionLabel = pinned ? "拖动模式" : "固定模式";
  pinButton.classList.toggle("is-active", pinned);
  pinButton.setAttribute("aria-pressed", String(pinned));
  pinButton.setAttribute("aria-label", actionLabel);
  pinButton.title = actionLabel;

  if (pinned) {
    boardHeaderElement.removeAttribute("data-tauri-drag-region");
  } else {
    boardHeaderElement.setAttribute("data-tauri-drag-region", "deep");
  }
}

/** 关闭按钮跟随“关闭时最小化到托盘”设置：开启显示最小化图标，关闭显示退出图标。 */
function applyCloseAction(closeToTray: boolean): void {
  closeIconElement.toggleAttribute("hidden", closeToTray);
  minimizeIconElement.toggleAttribute("hidden", !closeToTray);
  closeButton.classList.toggle("is-minimize", closeToTray);

  const actionLabel = closeToTray ? "最小化" : "退出";
  closeButton.setAttribute("aria-label", actionLabel);
  closeButton.title = actionLabel;
}

/** 通知消息与托盘动效由 Rust 后台线程驱动，设置变化时同步过去；浏览器预览无事可做。 */
function syncActivityPreferences(
  systemNotifications: boolean,
  trayWorkingBreathing: boolean,
  trayAlertFlicker: boolean,
): void {
  if (!IS_TAURI) {
    return;
  }
  void setActivityPreferences(systemNotifications, trayWorkingBreathing, trayAlertFlicker).catch(
    (error: unknown) => {
      console.warn("pinglet: 同步通知与托盘动效偏好失败", error);
    },
  );
}

async function resetAllSettings(): Promise<void> {
  const defaults = resetSettings();
  applyPinned(defaults.pinned);
  // 恢复默认会立即应用穿透状态，丢弃面板内尚未生效的勾选
  isClickThroughPending = false;

  if (IS_TAURI) {
    const tasks: Array<{ label: string; run: Promise<void> }> = [
      { label: "关闭行为", run: setCloseToTray(defaults.closeToTray) },
      { label: "鼠标穿透", run: setClickThrough(defaults.clickThrough) },
      { label: "开机自启", run: setAutoStart(defaults.autoStart) },
      {
        label: "全局快捷键",
        run: setGlobalShortcut(defaults.shortcut === "" ? null : defaults.shortcut),
      },
      {
        label: "通知与托盘动效偏好",
        run: setActivityPreferences(
          defaults.systemNotifications,
          defaults.trayEffects.workingBreathing,
          defaults.trayEffects.alertFlicker,
        ),
      },
      { label: "窗口位置", run: resetWindowPosition() },
    ];
    const results = await Promise.allSettled(tasks.map((task) => task.run));
    const failures = results.flatMap((result, index) =>
      result.status === "rejected" ? [`${tasks[index].label}（${toMessage(result.reason)}）`] : [],
    );
    if (failures.length > 0) {
      throw new Error(failures.join("；"));
    }
  }

  // 清空全部本地偏好（设置与窗口尺寸），刷新后回到默认宽高、字号与主题
  if (windowSizeSaveTimer !== undefined) {
    window.clearTimeout(windowSizeSaveTimer);
    windowSizeSaveTimer = undefined;
  }
  try {
    window.localStorage.clear();
  } catch (error) {
    console.warn("pinglet: 清空本地存储失败", error);
  }

  preferredWindowWidth = DEFAULT_WINDOW_WIDTH;
  preferredWindowHeight = clampWindowHeight(DEFAULT_WINDOW_HEIGHT, getMaxWindowHeight());
  await applyWindowSize(
    preferredWindowWidth,
    clampWindowHeight(Math.min(window.innerHeight, preferredWindowHeight), getMaxWindowHeight()),
  );

  window.setTimeout(() => {
    window.location.reload();
  }, RESET_RELOAD_DELAY_MS);
}

function handleClickThroughChange(enabled: boolean): void {
  // 关闭穿透是安全操作，立即生效；开启先挂起，等设置面板关闭后再应用
  if (!enabled) {
    isClickThroughPending = false;
    void setClickThrough(false).catch((error: unknown) => {
      setClickThroughSetting(true);
      showNotice(`关闭鼠标穿透失败: ${toMessage(error)}`);
    });
    return;
  }
  isClickThroughPending = true;
}

function applyPendingClickThrough(): void {
  if (!isClickThroughPending) {
    return;
  }
  isClickThroughPending = false;
  void setClickThrough(true)
    .then(() => {
      const shortcut = getSettings().shortcut;
      showNotice(
        shortcut === ""
          ? "已开启鼠标穿透；可在托盘菜单「鼠标穿透」中恢复"
          : `已开启鼠标穿透；按 ${shortcut} 或托盘菜单可恢复`,
      );
    })
    .catch((error: unknown) => {
      setClickThroughSetting(false);
      showNotice(`鼠标穿透设置失败: ${toMessage(error)}`);
    });
}

function setSettingsOpen(open: boolean): void {
  if (isSettingsOpen === open) {
    return;
  }

  isSettingsOpen = open;
  settingsPanelElement.hidden = !open;
  settingsBackdropElement.hidden = !open;
  panelElement.classList.toggle("settings-open", open);
  settingsButton.setAttribute("aria-expanded", String(open));
  settingsButton.classList.toggle("is-active", open);
  hideTooltip();
  hideLogoTooltip();
  hideRowMenu();

  if (open) {
    void settingsPanelHandle.syncSystemState();
    settingsPanelElement.querySelector<HTMLButtonElement>("button")?.focus();
  } else {
    settingsButton.focus();
    applyPendingClickThrough();
  }

  window.requestAnimationFrame(() => {
    void resizeWindowToContent();
  });
}

// 拖拽区域内的按钮需要阻断 mousedown，否则会被窗口拖拽抢占
function keepClickableInDragRegion(button: HTMLButtonElement): void {
  button.addEventListener("mousedown", (event) => {
    event.stopPropagation();
  });
}

keepClickableInDragRegion(pinButton);
keepClickableInDragRegion(settingsButton);
keepClickableInDragRegion(closeButton);
setupResizeHandles();

logoElement.addEventListener("mouseenter", () => {
  hideTooltip();
  isLogoHovered = true;
  scheduleLogoTooltip();
});
logoElement.addEventListener("mouseleave", hideLogoTooltip);
logoElement.addEventListener("pointerdown", hideLogoTooltip);

pinButton.addEventListener("click", () => {
  const pinned = !getSettings().pinned;
  setPinnedSetting(pinned);
  applyPinned(pinned);
});

settingsButton.addEventListener("click", () => {
  setSettingsOpen(!isSettingsOpen);
});

settingsBackdropElement.addEventListener("click", () => {
  setSettingsOpen(false);
});

closeButton.addEventListener("click", () => {
  if (!IS_TAURI) {
    showNotice("浏览器预览模式：关闭窗口不可用");
    return;
  }
  // 关闭 / 最小化前让挂起的穿透生效，避免设置与窗口实际状态不一致
  applyPendingClickThrough();
  markPanelHidden(panelAnimation);
  startPanelHideAnimation(() => {
    void closeWindow().catch((error: unknown) => {
      showNotice(`关闭窗口失败: ${toMessage(error)}`);
      expandPanel();
    });
  });
});

document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") {
    return;
  }
  if (rowMenuElement !== null && !rowMenuElement.hidden) {
    hideRowMenu();
    return;
  }
  if (isSettingsOpen) {
    setSettingsOpen(false);
  }
});

document.addEventListener("pointerdown", unlockAudio, { once: true });
document.addEventListener("pointerdown", (event) => {
  if (rowMenuElement === null || rowMenuElement.hidden) {
    return;
  }
  if (event.target instanceof Node && rowMenuElement.contains(event.target)) {
    return;
  }
  hideRowMenu();
});
listElement.addEventListener(
  "scroll",
  () => {
    hideTooltip();
    hideRowMenu();
    revealScrollbar();
  },
  { passive: true },
);
panelElement.addEventListener("contextmenu", (event) => {
  event.preventDefault();
});

const settingsPanelHandle = mountSettingsPanel(settingsPanelElement, {
  onNotice: showNotice,
  onPreviewReminder: previewReminder,
  onResetSettings: resetAllSettings,
  onClickThroughChange: handleClickThroughChange,
  onRestoreHidden: restoreHiddenSessions,
  onClose: () => {
    setSettingsOpen(false);
  },
});
hiddenCountListener = settingsPanelHandle.setHiddenSessionCount;

const initialSettings = getSettings();
let lastThemeMode = initialSettings.theme;
applyPinned(initialSettings.pinned);
applyCloseAction(initialSettings.closeToTray);
syncActivityPreferences(
  initialSettings.systemNotifications,
  initialSettings.trayEffects.workingBreathing,
  initialSettings.trayEffects.alertFlicker,
);
startAutoTheme();
window.setTimeout(() => {
  setThemeTransitionEnabled(true);
}, THEME_TRANSITION_ENABLE_DELAY_MS);
subscribeSettings(() => {
  const settings = getSettings();
  applyCloseAction(settings.closeToTray);
  // 只有主题模式变化才重启自动主题采样，避免无关设置反复重置判定进度
  if (settings.theme !== lastThemeMode) {
    lastThemeMode = settings.theme;
    startAutoTheme();
  }
  syncActivityPreferences(
    settings.systemNotifications,
    settings.trayEffects.workingBreathing,
    settings.trayEffects.alertFlicker,
  );
  // 字号变化会改变内容高度，下一帧重新按内容调整窗口
  window.requestAnimationFrame(() => {
    void resizeWindowToContent();
  });
});
if (IS_TAURI) {
  initialWindowSizeReady = applyInitialWindowSize();
  void onWindowResized(handleWindowResized).catch((error: unknown) => {
    console.warn("pinglet: 监听窗口尺寸失败", error);
  });
  // Rust 侧判断窗口是否可见/最小化；恢复后立即刷新一次并重新启动自动主题采样
  void onWindowVisibility((active) => {
    isWindowActive = active;
    syncWindowPauseClass();
    if (active) {
      expandPanel();
      void refresh();
      startAutoTheme();
    } else {
      // 隐藏/最小化时没有可见主题可调，停掉周期采样，重新显示时再立即判定
      handleWindowHidden();
      stopAutoTheme();
    }
  }).catch((error: unknown) => {
    console.warn("pinglet: 监听窗口显隐失败", error);
  });
  void onWindowHideRequest(handleWindowHideRequest).catch((error: unknown) => {
    console.warn("pinglet: 监听隐藏动画请求失败", error);
  });
  void onWindowShowRequest(() => {
    expandPanel();
  }).catch((error: unknown) => {
    console.warn("pinglet: 监听显示动画请求失败", error);
  });
  void onWindowMoved(() => {
    scheduleAutoThemeSample();
  }).catch((error: unknown) => {
    console.warn("pinglet: 监听窗口移动失败", error);
  });
  void setCloseToTray(initialSettings.closeToTray).catch((error: unknown) => {
    console.warn("pinglet: 初始化关闭行为失败", error);
  });
  void setClickThrough(initialSettings.clickThrough).catch((error: unknown) => {
    console.warn("pinglet: 初始化鼠标穿透失败", error);
  });
  void onTrayClickThroughChange((enabled) => {
    setClickThroughSetting(enabled);
  }).catch((error: unknown) => {
    console.warn("pinglet: 监听托盘穿透状态失败", error);
  });
  void onOpenSettings(() => {
    setSettingsOpen(true);
  }).catch((error: unknown) => {
    console.warn("pinglet: 监听托盘设置入口失败", error);
  });
  if (initialSettings.shortcut !== "") {
    void setGlobalShortcut(initialSettings.shortcut).catch((error: unknown) => {
      showNotice(`全局快捷键注册失败: ${toMessage(error)}`);
    });
  }
  void settingsPanelHandle.syncSystemState();
}

if (
  import.meta.env.DEV &&
  ["1", "settings"].includes(
    new URLSearchParams(window.location.search).get("settings") ??
      (new URLSearchParams(window.location.search).get("theme") === "1" ? "1" : ""),
  )
) {
  setSettingsOpen(true);
}

// 窗口隐藏 / 最小化时暂停轮询；Rust 事件与页面可见性任一判定隐藏即暂停
function isPollingPaused(): boolean {
  return !isWindowActive || document.visibilityState === "hidden";
}

/** 同步暂停持续动效的根节点类；窗口不可见时 CSS 动画全部暂停。 */
function syncWindowPauseClass(): void {
  document.documentElement.classList.toggle("is-window-paused", isPollingPaused());
}

syncWindowPauseClass();
updateHeartbeat();
setInterval(() => {
  if (isPollingPaused()) {
    return;
  }
  updateHeartbeat();
}, HEARTBEAT_INTERVAL_MS);
setInterval(() => {
  if (isPollingPaused()) {
    return;
  }
  void refresh();
}, REFRESH_INTERVAL_MS);
document.addEventListener("visibilitychange", () => {
  syncWindowPauseClass();
  if (document.visibilityState === "visible" && isWindowActive) {
    void refresh();
  }
});

void refresh();
