/**
 * @ai-generated deepseek-v4-flash
 * 自绘通知弹窗：渲染 Rust 侧暂存的告警摘要，短暂停留后淡出并隐藏。
 * 仅桌面应用可用；浏览器预览（npm run dev）使用内置示例数据。
 */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { reloadSettingsFromStorage } from "./settings";

type NotificationItemKind = "blocked" | "interrupted";

type NotificationItem = {
  title: string;
  kind: NotificationItemKind;
  status: string;
};

type NotificationPayload = {
  title: string;
  items: NotificationItem[];
  more: string | null;
};

const EVENT_NOTIFICATION_SHOW = "pinglet://notification-show";
/** 卡片完整停留时长；Rust 侧另有 5 秒兜底隐藏。 */
const AUTO_HIDE_MS = 3000;
/** 淡出动画时长，提前于真正隐藏触发。 */
const LEAVE_ANIMATION_MS = 240;
/** body 四周留白，与 notification.css 的 padding 保持一致。 */
const CARD_PADDING_LOGICAL = 10;
const DEMO_PAYLOAD: NotificationPayload = {
  title: "Pinglet · 2 个会话需要关注",
  items: [
    { title: "api-server", kind: "blocked", status: "等待确认" },
    { title: "web-client", kind: "interrupted", status: "已中断" },
  ],
  more: null,
};

const card = getElement<HTMLElement>("notification-card");
const titleElement = getElement<HTMLElement>("notification-title");
const itemsElement = getElement<HTMLElement>("notification-items");
const moreElement = getElement<HTMLElement>("notification-more");
const closeButton = getElement<HTMLButtonElement>("notification-close");

let hideTimer: number | undefined;
let leaveTimer: number | undefined;

function getElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (element === null) {
    throw new Error(`通知弹窗缺少元素 #${id}`);
  }
  return element as T;
}

function render(payload: NotificationPayload): void {
  titleElement.textContent = payload.title;

  const rows = payload.items.map((item) => {
    const row = document.createElement("div");
    row.className = "notification-item";
    row.dataset.kind = item.kind;

    const itemTitle = document.createElement("span");
    itemTitle.className = "notification-item-title";
    itemTitle.textContent = item.title;

    const status = document.createElement("span");
    status.className = "notification-item-status";
    status.textContent = item.status;

    row.append(itemTitle, status);
    return row;
  });
  itemsElement.replaceChildren(...rows);

  moreElement.hidden = payload.more === null;
  moreElement.textContent = payload.more ?? "";
}

function restartAutoHide(): void {
  window.clearTimeout(hideTimer);
  window.clearTimeout(leaveTimer);
  card.classList.remove("is-leaving", "is-entering");
  // NOTE: 读取布局强制重排，保证窗口复用后进入动画能重新播放。
  void card.offsetWidth;
  card.classList.add("is-entering");
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

async function loadPayload(): Promise<void> {
  if (!isTauri()) {
    render(DEMO_PAYLOAD);
    return;
  }

  try {
    const payload = await invoke<NotificationPayload | null>("get_notification_payload");
    if (payload !== null) {
      render(payload);
    }
  } catch (error) {
    console.error("pinglet: 读取通知内容失败", error);
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
  await loadPayload();
  await resizeToContent();
  restartAutoHide();
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

closeButton.addEventListener("click", (event) => {
  event.stopPropagation();
  void hidePopup();
});

card.addEventListener("click", () => {
  void invoke("open_main_window").catch((error: unknown) => {
    console.error("pinglet: 打开主窗口失败", error);
  });
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    void hidePopup();
  }
});

void start();
