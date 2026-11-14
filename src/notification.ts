/**
 * @ai-generated deepseek-v4-flash
 * 自绘通知弹窗：每条告警独立成卡，点击卡片直接激活对应会话；
 * 鼠标悬浮托盘时由 Rust 侧重新展示当前未处理告警。
 * 仅桌面应用可用；浏览器预览（npm run dev）使用内置示例数据。
 */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CLI_ICON_SVG, VSCODE_ICON_SVG } from "./entry-icons";
import { assignFolderColorSlots, folderColorKey, folderTagColor } from "./folder-colors";
import { directoryName, entrypointLabel } from "./session-state";
import { reloadSettingsFromStorage } from "./settings";

type NotificationItemKind = "blocked" | "interrupted";

type NotificationItem = {
  key: string;
  title: string;
  kind: NotificationItemKind;
  status: string;
  /** 以下字段只用于点击激活会话，不渲染到卡片上。 */
  cwd: string | null;
  entrypoint: string | null;
  name: string;
  pid: number | null;
};

type NotificationPayload = {
  items: NotificationItem[];
  more: string | null;
  autoHide: boolean;
};

const EVENT_NOTIFICATION_SHOW = "pinglet://notification-show";
/** 卡片完整停留时长；Rust 侧另有 8 秒兜底隐藏。 */
const AUTO_HIDE_MS = 5000;
/** 淡出动画时长，提前于真正隐藏触发。 */
const LEAVE_ANIMATION_MS = 240;
/** body 四周留白，与 notification.css 的 padding 保持一致，用于容纳卡片阴影。 */
const CARD_PADDING_LOGICAL = 3;
const DEMO_PAYLOAD: NotificationPayload = {
  items: [
    {
      key: "demo-1",
      title: "api-server",
      kind: "blocked",
      status: "等待确认",
      cwd: "C:\\work\\api-server",
      entrypoint: "claude",
      name: "api-server",
      pid: 1,
    },
    {
      key: "demo-2",
      title: "web-client",
      kind: "interrupted",
      status: "已中断",
      cwd: "C:\\work\\web-client",
      entrypoint: "vscode",
      name: "web-client",
      pid: 2,
    },
  ],
  more: null,
  autoHide: false,
};

const card = getElement<HTMLElement>("notification-card");
const itemsElement = getElement<HTMLElement>("notification-items");
const moreElement = getElement<HTMLElement>("notification-more");

let hideTimer: number | undefined;
let leaveTimer: number | undefined;

function getElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (element === null) {
    throw new Error(`通知弹窗缺少元素 #${id}`);
  }
  return element as T;
}

async function activateItem(item: NotificationItem): Promise<void> {
  if (!isTauri() || item.cwd === null) {
    return;
  }
  try {
    await invoke("activate_session", {
      cwd: item.cwd,
      entrypoint: item.entrypoint,
      name: item.name,
      pid: item.pid,
    });
    await hidePopup();
  } catch (error) {
    console.error("pinglet: 激活通知会话失败", error);
  }
}

function render(payload: NotificationPayload): void {
  // 目录标签复用会话列表的哈希配色：每次渲染按当前卡片重新分配槽位
  const folderSlots = assignFolderColorSlots(
    payload.items.map((item) => folderColorKey(item.cwd)),
    new Map(),
  );

  const rows = payload.items.map((item) => {
    const row = document.createElement("button");
    row.type = "button";
    row.className = "notification-item";
    row.dataset.kind = item.kind;
    row.title = "激活该会话";

    // 第一行与会话列表一致：状态灯、状态、目录标签、入口图标
    const mainLine = document.createElement("span");
    mainLine.className = "notification-item-main";

    const dot = document.createElement("span");
    dot.className = "notification-item-dot";
    dot.setAttribute("aria-hidden", "true");

    const status = document.createElement("span");
    status.className = "notification-item-status";
    status.textContent = item.status;

    const directory = document.createElement("span");
    directory.className = "notification-item-meta";
    directory.textContent = directoryName(item.cwd ?? null);
    directory.style.setProperty(
      "--tag-color",
      folderTagColor(folderColorKey(item.cwd), folderSlots),
    );

    mainLine.append(dot, status, directory);

    const entry = entrypointLabel(item.entrypoint);
    const iconKind = entry === "vs-code" || entry === "cli" ? entry : null;
    if (iconKind !== null) {
      const entryIcon = document.createElement("span");
      entryIcon.className = "entry-icon";
      entryIcon.dataset.kind = iconKind;
      entryIcon.setAttribute("aria-hidden", "true");
      entryIcon.innerHTML = iconKind === "vs-code" ? VSCODE_ICON_SVG : CLI_ICON_SVG;
      mainLine.append(entryIcon);
    }

    // 第二行是会话标题，独占整行，短标题也不会在行内留下大段空白
    const title = document.createElement("span");
    title.className = "notification-item-title";
    title.textContent = item.title;

    row.append(mainLine, title);
    row.addEventListener("click", (event) => {
      event.stopPropagation();
      void activateItem(item);
    });
    return row;
  });
  itemsElement.replaceChildren(...rows);

  moreElement.hidden = payload.more === null;
  moreElement.textContent = payload.more ?? "";
}

function playEnterAnimation(): void {
  window.clearTimeout(hideTimer);
  window.clearTimeout(leaveTimer);
  card.classList.remove("is-leaving", "is-entering");
  // NOTE: 读取布局强制重排，保证窗口复用后进入动画能重新播放。
  void card.offsetWidth;
  card.classList.add("is-entering");
}

function restartAutoHide(): void {
  playEnterAnimation();
  leaveTimer = window.setTimeout(() => {
    card.classList.add("is-leaving");
  }, AUTO_HIDE_MS - LEAVE_ANIMATION_MS);
  hideTimer = window.setTimeout(() => {
    void hidePopup();
  }, AUTO_HIDE_MS);
}

async function hidePopup(): Promise<void> {
  window.clearTimeout(hideTimer);
  window.clearTimeout(leaveTimer);
  try {
    await invoke("hide_notification_window");
  } catch (error) {
    console.error("pinglet: 隐藏通知弹窗失败", error);
  }
}

async function loadPayload(): Promise<NotificationPayload | null> {
  if (!isTauri()) {
    return DEMO_PAYLOAD;
  }

  try {
    return await invoke<NotificationPayload | null>("get_notification_payload");
  } catch (error) {
    console.error("pinglet: 读取通知内容失败", error);
    return null;
  }
}

/** 按卡片实际高度收缩窗口，避免透明空白区域拦截下方窗口的点击。 */
async function resizeToContent(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  card.classList.remove("is-entering", "is-leaving");
  const height = Math.ceil(card.getBoundingClientRect().height + CARD_PADDING_LOGICAL * 2);
  try {
    await invoke("resize_notification_window", { height });
  } catch (error) {
    console.error("pinglet: 调整通知弹窗尺寸失败", error);
  }
}

async function showPayload(): Promise<void> {
  // 跟随主窗口当前主题与字号：自动模式下复用主窗口持久化的生效结果
  reloadSettingsFromStorage();
  const payload = await loadPayload();
  if (payload === null || payload.items.length === 0) {
    // 没有可展示内容时不要留下空白透明窗口
    await hidePopup();
    return;
  }
  render(payload);
  await resizeToContent();
  if (payload.autoHide) {
    restartAutoHide();
  } else {
    // 托盘悬浮触发的列表由鼠标离开托盘/弹窗后延迟隐藏，不启动自动倒计时
    playEnterAnimation();
  }
}

async function start(): Promise<void> {
  if (isTauri()) {
    try {
      await listen(EVENT_NOTIFICATION_SHOW, () => {
        void showPayload();
      });
    } catch (error) {
      console.error("pinglet: 监听通知内容事件失败", error);
    }
  }

  await showPayload();
}

function reportHover(hovered: boolean): void {
  if (!isTauri()) {
    return;
  }
  void invoke("set_notification_hover", { hovered }).catch((error: unknown) => {
    console.error("pinglet: 同步通知悬浮状态失败", error);
  });
}

card.addEventListener("mouseenter", () => {
  // 悬浮查看期间取消自动隐藏，避免卡片在阅读时消失
  window.clearTimeout(hideTimer);
  window.clearTimeout(leaveTimer);
  card.classList.remove("is-leaving");
  reportHover(true);
});

card.addEventListener("mouseleave", () => {
  reportHover(false);
  if (!isTauri()) {
    return;
  }
  void invoke("schedule_notification_hide").catch((error: unknown) => {
    console.error("pinglet: 延迟隐藏通知弹窗失败", error);
  });
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    void hidePopup();
  }
});

void start();
