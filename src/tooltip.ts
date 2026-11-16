/**
 * @ai-generated deepseek-v4-flash
 * 会话悬浮详情独立窗口：按主窗口传来的内容渲染，实测尺寸后由 Rust 定位并显示。
 */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { reloadSettingsFromStorage } from "./settings";

const EVENT_TOOLTIP_SHOW = "pinglet://tooltip-show";

type TooltipPayload = {
  title: string;
  lines: string[];
  hint: string | null;
};

type TooltipSnapshot = {
  payload: TooltipPayload;
  generation: number;
};

const tooltipElement = getElement<HTMLDivElement>("tooltip");
const tooltipTitleElement = getElement<HTMLDivElement>("tooltip-title");
const tooltipBodyElement = getElement<HTMLDivElement>("tooltip-body");

let isRendering = false;
let shouldReshow = false;

function getElement<T extends HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (element === null) {
    throw new Error(`悬浮详情缺少元素 #${id}`);
  }
  return element as T;
}

function render(payload: TooltipPayload): void {
  tooltipTitleElement.textContent = payload.title;
  const lineElements = payload.lines.map((line) => {
    const element = document.createElement("div");
    element.className = "tooltip-line";
    element.textContent = line;
    return element;
  });
  if (payload.hint !== null) {
    const hint = document.createElement("div");
    hint.className = "tooltip-hint";
    hint.textContent = payload.hint;
    lineElements.push(hint);
  }
  tooltipBodyElement.replaceChildren(...lineElements);
}

/** 渲染并上报实测尺寸；Rust 校验代号后按鼠标位置定位并显示窗口。 */
async function renderOnce(): Promise<void> {
  // 跟随主窗口当前主题与字号：自动模式下复用主窗口持久化的生效结果
  reloadSettingsFromStorage();
  const snapshot = await invoke<TooltipSnapshot | null>("get_tooltip_payload");
  if (snapshot === null) {
    return;
  }

  render(snapshot.payload);
  const rect = tooltipElement.getBoundingClientRect();
  await invoke("resize_tooltip_window", {
    generation: snapshot.generation,
    width: Math.ceil(rect.width),
    height: Math.ceil(rect.height),
  });
}

async function show(): Promise<void> {
  if (!isTauri()) {
    return;
  }
  // NOTE: 事件可能在上一轮渲染尚未结束时到达，标记后重跑一轮读取最新内容。
  if (isRendering) {
    shouldReshow = true;
    return;
  }

  isRendering = true;
  try {
    do {
      shouldReshow = false;
      await renderOnce();
    } while (shouldReshow);
  } catch (error) {
    console.error("pinglet: 渲染悬浮详情失败", error);
  } finally {
    isRendering = false;
  }
}

async function start(): Promise<void> {
  if (!isTauri()) {
    return;
  }
  try {
    await listen(EVENT_TOOLTIP_SHOW, () => {
      void show();
    });
  } catch (error) {
    console.error("pinglet: 监听悬浮详情失败", error);
  }
  await show();
}

void start();
