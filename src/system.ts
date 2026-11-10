/**
 * @ai-generated deepseek-v4-flash
 * Tauri 能力封装：窗口尺寸、置顶、关闭、托盘关闭行为、鼠标穿透、开机自启、全局快捷键与打开会话。
 */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { LogicalSize, PhysicalPosition } from "@tauri-apps/api/dpi";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import type { ListResult } from "./session-state";

// NOTE: isTauri() 在个别版本可能未注入，__TAURI_INTERNALS__ 是 Tauri 2 的稳定标志
export const IS_TAURI = isTauri() || "__TAURI_INTERNALS__" in window;

// Tauri 的 ResizeDirection 未从模块导出，这里按相同字面量声明
export type ResizeDirection =
  "East" | "North" | "NorthEast" | "NorthWest" | "South" | "SouthEast" | "SouthWest" | "West";

function requireTauri(): void {
  if (!IS_TAURI) {
    throw new Error("仅桌面应用可用");
  }
}

export function toMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export async function listSessions(): Promise<ListResult> {
  requireTauri();
  return invoke<ListResult>("list_sessions");
}

export async function resizeWindow(width: number, height: number): Promise<void> {
  requireTauri();
  await getCurrentWindow().setSize(new LogicalSize(width, height));
}

/** 无边框窗口的边缘/角落热区按下后，交给系统执行缩放拖拽。 */
export async function startResizeDragging(direction: ResizeDirection): Promise<void> {
  requireTauri();
  await getCurrentWindow().startResizeDragging(direction);
}

export async function onWindowResized(
  handler: (physicalWidth: number, physicalHeight: number) => void,
): Promise<UnlistenFn> {
  if (!IS_TAURI) {
    return () => {};
  }
  return getCurrentWindow().onResized(({ payload }) => {
    handler(payload.width, payload.height);
  });
}

/** 恢复持久化宽度后窗口可能越出工作区，启动时向左/上推回可见范围。 */
export async function keepWindowInsideWorkArea(): Promise<void> {
  requireTauri();
  const currentWindow = getCurrentWindow();
  const [monitor, position, size] = await Promise.all([
    currentMonitor(),
    currentWindow.outerPosition(),
    currentWindow.outerSize(),
  ]);
  if (monitor === null) {
    return;
  }

  const { workArea } = monitor;
  const x = Math.max(
    workArea.position.x,
    Math.min(workArea.position.x + workArea.size.width - size.width, position.x),
  );
  const y = Math.max(
    workArea.position.y,
    Math.min(workArea.position.y + workArea.size.height - size.height, position.y),
  );
  if (x === position.x && y === position.y) {
    return;
  }
  await currentWindow.setPosition(new PhysicalPosition(x, y));
}

export async function closeWindow(): Promise<void> {
  requireTauri();
  await getCurrentWindow().close();
}

export async function quitApp(): Promise<void> {
  requireTauri();
  await invoke("quit_app");
}

export async function setCloseToTray(enabled: boolean): Promise<void> {
  requireTauri();
  await invoke("set_close_to_tray", { enabled });
}

export async function setClickThrough(enabled: boolean): Promise<void> {
  requireTauri();
  await invoke("set_click_through", { enabled });
}

/** 监听托盘菜单对穿透模式的切换，保持设置面板与托盘状态一致。 */
export async function onTrayClickThroughChange(
  handler: (enabled: boolean) => void,
): Promise<UnlistenFn> {
  if (!IS_TAURI) {
    return () => {};
  }
  return listen<boolean>("pinglet://click-through-changed", (event) => {
    handler(event.payload);
  });
}

/** 监听托盘「设置」菜单，打开设置面板。 */
export async function onOpenSettings(handler: () => void): Promise<UnlistenFn> {
  if (!IS_TAURI) {
    return () => {};
  }
  return listen("pinglet://open-settings", () => {
    handler();
  });
}

/**
 * 监听主窗口显隐（Rust 侧为权威来源）；隐藏时前端暂停轮询与心跳。
 * NOTE: Windows 上 WebView2 会节流隐藏窗口的后台计时器，不能只依赖 visibilitychange。
 */
export async function onWindowVisibility(handler: (active: boolean) => void): Promise<UnlistenFn> {
  if (!IS_TAURI) {
    return () => {};
  }
  return listen<boolean>("pinglet://window-visibility", (event) => {
    handler(event.payload);
  });
}

/** 监听窗口移动结束，用于自动主题重新采样。 */
export async function onWindowMoved(handler: () => void): Promise<UnlistenFn> {
  if (!IS_TAURI) {
    return () => {};
  }
  return listen("pinglet://window-moved", () => {
    handler();
  });
}

export async function sampleBackdropLuminance(): Promise<number | null> {
  if (!IS_TAURI) {
    return null;
  }
  return invoke<number | null>("sample_backdrop_luminance");
}

export async function setNotificationPreferences(
  systemNotifications: boolean,
  doNotDisturb: boolean,
): Promise<void> {
  requireTauri();
  await invoke("set_notification_preferences", { systemNotifications, doNotDisturb });
}

/** 恢复默认窗口位置并刷新 window-state 存档，供「恢复默认」使用。 */
export async function resetWindowPosition(): Promise<void> {
  requireTauri();
  await invoke("reset_window_position");
}

export async function getAutoStart(): Promise<boolean> {
  requireTauri();
  return invoke<boolean>("is_autostart_enabled");
}

export async function setAutoStart(enabled: boolean): Promise<void> {
  requireTauri();
  await invoke("set_autostart", { enabled });
}

export async function setGlobalShortcut(shortcut: string | null): Promise<void> {
  requireTauri();
  await invoke("set_global_shortcut", { shortcut });
}

export async function activateSession(
  cwd: string,
  entrypoint: string | null,
  name: string,
  pid: number | null,
): Promise<void> {
  requireTauri();
  await invoke("activate_session", { cwd, entrypoint, name, pid });
}
